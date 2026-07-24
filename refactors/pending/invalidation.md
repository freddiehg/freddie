# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root; mutation is free. One `&mut Context` is threaded up the ascent and mutated in place. **Context holds `depth`** (remaining `into_parent` hops still inside a destroyed region) and **claim** (exclusive try-take). Posts read validity through the getter **`validity()`** — not a stored field; pure view of `depth == 0`. `#[bind]` is a post with no pre, gated by `ctx.claim()`.

**Generate stays thin.** The derive only schedules `opt_N` and calls helpers (`run_post`, `run_exclusive`, `into_parent`). Depth math, claim try-take, and sink extension live in ordinary functions in `bind` / laserbeam — not hand-rolled in every expanded `Dispatch` impl.

## Paths: shared down, owned up

The path type is the normal owned `PathMut` (same as today). What changes is when the handler may hold it.

- **Descent (pre):** the framework still owns the path for the walk. Pre gets a shared borrow: `Node<&P, D>`. Read-only; no reshape.
- **Ascent (post / bind):** `into_parent` has recovered this level's path as an owned `P` again (`from_fn` down, `into_parent` up). Post and bind receive that owned path: `Node<P, D>`. `get_mut`, `ascend_mut`, the usual tools.

No parallel "`&mut Path`" API. Mutation is allowed because ownership of the normal path is back at this level on the way up.

To thread ownership through several posts at one level, each post returns the path with its effects: `(Vec<Effect>, P)`. The value carried from pre to post is whatever pre returns — a concrete type, inferred, not a type parameter of the framework.

## Developer experience

```rust
#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(AnyKey => (snap_child_id, after_child))]
#[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
#[bind(KeyA => outer_handler)]
struct Outer {
    #[resolve_into]
    inner: Inner,
    return_home: AndReturnHome,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner {
    id: ChildId,
}
```

```rust
// pre: shared path, read-only. Snapshot what the ascent may destroy.
fn snap_child_id(ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

// post: owned path; first arg is pre's return. Must not call ctx.claim().
// Pre carriage exists so Invalidated still has the id after the child field is gone.
fn after_child(
    id: ChildId,
    node: Node<OuterPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, OuterPath) {
    match ctx.validity() {
        Validity::Valid => {
            // child field still Inner; id should match live value
            let _ = (id, node.parent.get().inner.id);
            (vec![], node.parent)
        }
        Validity::Invalidated => {
            // field no longer Inner; only the pre snapshot remains
            (vec![log_destroyed(id)], node.parent)
        }
    }
}

// #[post] alone: (noop_pre, body). User post is (node, ctx) — not ((), node, ctx).
// sugar: project + act only when validity is Valid
fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

// well-known; macro drops this in for #[post] / #[bind] when there is no pre
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

// bind: event + node + ctx. Gating is run_exclusive at the call site.
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, InnerPath) { ... }
```

Expression positions work as today (`#handler(…)` splice). Pinned by `crates/bind/tests/expr_handler.rs`.

## Semantics

### Schedule on the way down (final)

Every pre/post attr is a pre_post pair. The macro fills a missing pre with `noop_pre`:

- `#[pre_post(trig => (pre, post))]` → `(pre, post)`
- `#[post(trig => post)]` → `(noop_pre, post)`
- `#[bind(trig => h)]` → `(noop_pre, exclusive(h))`

There is no `#[pre]` alone. A pre whose return is only dropped on the ascent does nothing useful (pre is read-only and may not yet emit now-effects). When a pre exists, a user post consumes its return — that is `#[pre_post]`.

For each pair on the node, if the trigger matches: call the pre with `Node<&P, D>`, store `opt_N = Some(pre_return)`. Miss → `None`. Ascent never re-checks triggers.

- `noop_pre` returns `()` — schedule token that the user post does **not** receive (generate calls the user body as `(node, ctx)` only).
- `#[bind]`: same schedule shape as `#[post]`; body still gets the dispatch event.

`N` is the attribute index on the node (`opt_0`, `opt_1`, …). Never names from triggers or handlers.

```rust
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> /* concrete type, inferred */
```

No reshape on the descent. Whether pre may also return now-effects is open; generate is return-value only.

### Execute on the way up (all scheduled posts run)

Leaf to root. One **`&mut Context`** for the whole ascent — same object, mutated as we go. No descent mutation of context.

`Context` is a small bag with two fields (same pointer the whole ascent):

| field | rule |
|---|---|
| `depth: u32` | Remaining hops inside a destroyed region. Lives **on Context**. A kill that climbs N `into_parent`s does `depth = depth.max(N)`; each later framework `into_parent` on the ascent calls `step_up` (decrement). `0` means valid. |
| `claim: Option<Claimed>` | Ascent-global, monotone. `claim()` try-takes. Once taken, every shallower exclusive fails. |

There is no stored `Validity` flag and no per-level context. The binary read is a getter: `fn validity(&self) -> Validity` over `depth == 0`. Binary `set_validity(Valid|Invalidated)` per level is wrong: not every level is invalidated, and a flag does not track how far destruction reaches.

Descent does not touch Context. Only the ascent mutates it.

### `d` is the into_parent chain length

A post or exclusive that destroys a spine does so by recovering ancestors with successive `into_parent` calls (today's `ascend_mut` + `set_layer` shape). **`d` is how many hops that chain took.**

```rust
// handler climbs two levels to replace a field at the owner
let path = path.into_parent(/* … */); // hop 1
let path = path.into_parent(/* … */); // hop 2
// …
// equivalent effect on Context:
ctx.depth = ctx.depth.max(2); // invalidate(2)
```

`into_parent().into_parent()` → `depth.max(2)`. One hop → `depth.max(1)`. Concurrent kills: still `max` (a deeper kill is not shrunk by a shallower one).

`invalidate(d)` is that assignment. Whether the hop count is applied inside each kill-side `into_parent` or once at the end of the chain is an implementation detail; the observable rule is `depth = depth.max(N)` for an N-hop climb.

At each level of the **framework** ascent (after the handler returns):

1. Apply reshape if scheduled for this level (may already have raised depth via the hop rule above).
2. Run scheduled posts (`&mut ctx`); they see **current** `ctx.depth` / `validity()`.
3. Exclusive: `match ctx.claim() { None => skip; Some(Claimed) => body }` — try-take, not a separate getter + set.
4. `step_up` when leaving the level (framework `into_parent` after this level's posts).

```rust
#[derive(Clone, Copy)]
enum Validity {
    Valid,
    Invalidated,
}

#[derive(Clone, Copy)]
struct Claimed;

struct Context {
    /// Remaining into_parent hops still inside the destroyed region. 0 = valid.
    depth: u32,
    claim: Option<Claimed>,
}

impl Context {
    fn depth(&self) -> u32 {
        self.depth
    }

    /// Getter. Valid iff depth == 0. Not a stored field.
    fn validity(&self) -> Validity {
        if self.depth == 0 {
            Validity::Valid
        } else {
            Validity::Invalidated
        }
    }

    /// Raise the invalidated zone by `d` hops (max with current).
    fn invalidate(&mut self, d: u32) {
        self.depth = self.depth.max(d);
    }

    /// One hop up. Called from into_parent after this level's posts.
    fn step_up(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Try to take exclusive ownership of this event.
    /// `Some(Claimed)` if it was open (now taken). `None` if already taken.
    fn claim(&mut self) -> Option<Claimed> {
        match self.claim {
            Some(_) => None,
            None => {
                self.claim = Some(Claimed);
                Some(Claimed)
            }
        }
    }
}
```

`claim` is not a getter. It claims. Logging / plain posts never call it.

### How the fields move

```text
ctx: depth = 0, claim = None

DESCENT: schedule opt_N only — Context untouched

ASCENT leaf → root:
  exclusive kills spine with N× into_parent → depth = depth.max(N)
  at each level:
    reshape if any (same hop rule if applied here)
    posts: read ctx.depth / validity()
    exclusive: match ctx.claim() { None => skip; Some(_) => h(...) }
    step_up: depth = depth.saturating_sub(1)
```

Example: exclusive two levels below the owner does `into_parent().into_parent()` → `depth.max(2)`.

```text
// after the kill, still at the leaf for sibling posts? or already returned path at owner —
// remaining framework ascent from the first hop still inside the zone:
level +1 posts: depth 2 → Invalidated; step_up → 1
level +2 owner: depth 1 → Invalidated; step_up → 0
above:          depth 0 → Valid
```

Three-hop climb (`into_parent`×3) → `depth.max(3)`, same pattern for three levels then Valid above.

`claim` never decrements. Depth only moves via `invalidate` (`max` with hop count) / `step_up`, and the counter lives on Context the whole time.

Posts return `(Vec<Effect>, P)` only.

### Defaults

All attrs desugar to a pre_post pair. Missing pre is well-known `noop_pre` the macro drops in:

```rust
// in bind — not generated, not per-node
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}
```

| attr | expands to | descent | ascent |
|---|---|---|---|
| `#[pre_post(t => (pre, post))]` | `(pre, post)` | `opt = Some(pre(…))` | `post(t, node, ctx)` |
| `#[post(t => post)]` | `(noop_pre, post)` | `opt = Some(noop_pre(…))` i.e. `Some(())` | `post(node, ctx)` — **not** `post((), node, ctx)` |
| `#[bind(t => h)]` | `(noop_pre, exclusive(h))` | same as post | `run_exclusive` + `h(ev, node, ctx)` |

No `#[pre]` alone. User posts never take a dummy `()` to drop. `noop_pre`'s `()` is only the schedule `Some`.

Several attrs on one node: `opt_0`, `opt_1`, … (indexed; each pair has its own concrete pre-return type), one `on_into_parent` closure.

### `#[bind]` is a post with no pre

```rust
#[bind(KeyA => outer_handler)]
// desugars to:
#[post(KeyA => exclusive(outer_handler))]
// exclusive gate is run_exclusive; body still takes the event.
```

```rust
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, OuterPath);

// Prefer naming: exclusive (role). if_unclaimed is the gate only and does not
// by itself say that a successful take mutates — claim() does that.
fn run_exclusive<P>(
    path: P,
    ctx: &mut Context,
    body: impl FnOnce(Node<P, ()>, &mut Context) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match ctx.claim() {
        None => (path, Vec::new()), // already taken
        Some(Claimed) => body(Node { parent: path, data: () }, ctx),
    }
}
```

Call sites by expand shape:

```rust
// #[pre_post] — thread real pre return
if let Some(id) = opt_0 {
    let (path, effects) = run_post(path, ctx, |node, ctx| after_child(id, node, ctx));
    Extend::extend(effs, effects);
}

// #[post] — macro filled noop_pre; user body does not take ()
if let Some(()) = opt_1 {
    let (path, effects) = run_post(path, ctx, |node, ctx| {
        only_if_valid(|p| &mut p.get_mut().return_home, rearm)(node, ctx)
    });
    Extend::extend(effs, effects);
}
```

Sibling posts never call `claim()`. Order at a level: pre_post / post attrs first (index order), then bind.

### `only_if_valid`

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, &mut Context) -> (Vec<Effect>, P) {
    move |mut node, ctx| {
        let effects = match ctx.validity() {
            Validity::Valid => f(project(&mut node.parent)),
            Validity::Invalidated => Vec::new(),
        };
        (effects, node.parent)
    }
}
```

## Types

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

#[derive(Clone, Copy)]
pub enum Validity {
    Valid,
    Invalidated,
}

pub struct Claimed;

pub struct Context {
    depth: u32,
    claim: Option<Claimed>,
}

impl Context {
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Getter. Valid iff depth == 0. Not a stored field.
    pub fn validity(&self) -> Validity {
        if self.depth == 0 {
            Validity::Valid
        } else {
            Validity::Invalidated
        }
    }

    pub fn invalidate(&mut self, d: u32) {
        self.depth = self.depth.max(d);
    }

    pub fn step_up(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Try-take. Some(Claimed) if open (now taken). None if already taken.
    pub fn claim(&mut self) -> Option<Claimed> {
        match self.claim {
            Some(_) => None,
            None => {
                self.claim = Some(Claimed);
                Some(Claimed)
            }
        }
    }
}

pub struct PathMut<N, P, F> {
    on_into_parent: F, // FnOnce(P, &mut Context) -> (P, Vec<Effect>)
}

// empty on_into_parent when this level has no scheduled posts
fn empty_on_into_parent<P>(parent: P, _ctx: &mut Context) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}

// missing pre half — well-known; macro drops in for #[post] / #[bind]
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, &mut Context) -> (P, Vec<Effect>),
{
    /// Reshape may invalidate(d); run posts; step_up; return parent.
    pub fn into_parent(self, sink: &mut Vec<Effect>, ctx: &mut Context) -> P {
        // if reshape applied here: ctx.invalidate(depth_for_this_kill)
        let (parent, post_effs) = (self.on_into_parent)(self.parent, ctx);
        Extend::extend(sink, post_effs);
        ctx.step_up();
        parent
    }
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        ctx: &mut Context,
    ) -> Self::Path<'a>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let mut ctx = Context {
        depth: 0,
        claim: None,
    };
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut ctx);
    // Do not call claim() again (try-take). Framework observes stored claim field.
    if /* claim field is Some */ || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

Handlers return `(Vec<Effect>, P)`. One `Context` for the ascent. Generate only calls helpers — no hand-rolled depth or claim logic in expanded node impls.

## Generated code

### Helpers (in bind, not generated)

```rust
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

fn run_post<P>(
    path: P,
    ctx: &mut Context,
    body: impl FnOnce(Node<P, ()>, &mut Context) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    body(Node { parent: path, data: () }, ctx)
}

fn run_exclusive<P>(
    path: P,
    ctx: &mut Context,
    body: impl FnOnce(Node<P, ()>, &mut Context) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match ctx.claim() {
        None => (path, Vec::new()), // already taken
        Some(Claimed) => body(Node { parent: path, data: () }, ctx),
    }
}
```

### Inner (leaf, one bind)

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        ctx: &mut Context,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        // index 0: #[bind(KeyA => inner_handler)]
        let opt_0 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(())
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        if let ::core::option::Option::Some(()) = opt_0 {
            let ev = /* &KeyEvent from event */;
            // body may ctx.invalidate(d) if it kills a spine
            let (path, out_effs) = run_exclusive(path, ctx, |node, ctx| {
                inner_handler(ev, node, ctx)
            });
            ::core::iter::Extend::extend(effs, out_effs);
            return path;
        }
        path
    }
}
```

### Outer (pre_post + post + bind)

```rust
impl Dispatch<M> for Outer {
    fn dispatch<'a>(
        mut path: <Outer as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        ctx: &mut Context,
    ) -> <Outer as Place>::Path<'a>
    where
        Self: 'a,
    {
        // ----- descent: schedule (opt_0, opt_1, opt_2 by attribute index) -----
        // opt_0: #[pre_post(AnyKey => (snap_child_id, after_child))]
        let opt_0 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = AnyKey;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(snap_child_id(
                    ev,
                    ::bind::Node {
                        parent: &path,
                        data: (),
                    },
                ))
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        // opt_1: #[post(AnyKey => only_if_valid(..., rearm))] via noop_pre
        let opt_1 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = AnyKey;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(noop_pre(
                    ev,
                    ::bind::Node {
                        parent: &path,
                        data: (),
                    },
                ))
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        // opt_2: #[bind(KeyA => outer_handler)] via noop_pre
        let opt_2 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(noop_pre(
                    ev,
                    ::bind::Node {
                        parent: &path,
                        data: (),
                    },
                ))
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, ctx| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = run_post(path, ctx, |node, ctx| after_child(id, node, ctx));
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(()) = opt_1 {
                    let (p, e) = run_post(path, ctx, |node, ctx| {
                        only_if_valid(|p| &mut p.get_mut().return_home, rearm)(node, ctx)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                (path, local)
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, ctx);

        // reshape / invalidate / posts / step_up inside into_parent
        let mut path = inner_path.into_parent(effs, ctx);

        if let ::core::option::Option::Some(()) = opt_2 {
            let ev = /* &KeyEvent from event */;
            let (p, e) = run_exclusive(path, ctx, |node, ctx| outer_handler(ev, node, ctx));
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }
        path
    }
}
```

### `#[post]` alone

Same indexed opts. Expands to `(noop_pre, post)`. User body is `(node, ctx)` (not given `()`). Never `claim()`.

## Walk

```text
DESCENT
  opt_0? opt_1? opt_2?
  move path into child

ASCENT  one &mut ctx (depth 0, claim None)
  Inner bind: may invalidate(d); run_exclusive via claim()
  Outer into_parent: posts see validity() from depth; step_up
  Outer bind: run_exclusive via claim()
```

### `KeyA` (matches AnyKey pre_post, AnyKey post, KeyA bind)

```text
DESCENT: snap_child_id; schedule rearm post; schedule outer bind
Inner claim() succeeds; may invalidate(d) and kill Inner
ASCENT Outer into_parent:
  after_child(id): Invalidated → log_destroyed(id) from pre snapshot
  only_if_valid rearm: skipped (Invalidated); Drop cancels old guard
Outer exclusive: claim already taken → skip
```

### `KeyB` (AnyKey only; no bind)

```text
after_child: Valid → child still live
rearm: Valid → new guard + schedule
never claim()
```

## Rearm

`#[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]` — see DX above.

`validity() == Valid`: rearm. `Invalidated` (depth > 0): skip; `Drop` of the guard cancels. Does not call `claim()`.

## Prefactors (ordered, each shippable alone)

Behavior-identical until a step says otherwise. No `#[post]` / `#[pre_post]` until feature steps. No completion token. No unused parameters "for later."

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

- `type Effect` is the item (`MercuryEffect`)
- `dispatch(path, event, effs: &mut Vec<Effect>) -> ControlFlow<(), Path>`
- exclusive pushes onto `effs` and `Break(())`
- top-level `Some(effs)` on win, `None` on miss
- `V: Into<Vec<Effect>>`; `From<MercuryEffect> for Vec<MercuryEffect>`

### P1 — `on_into_parent` + sink **together**

Do not add an unused sink before posts exist. Same change: `F` on `PathMut`, `into_parent` runs it and extends the sink. All sites pass `empty_on_into_parent` (empty effects). Behavior-identical.

### P2 — `from_fn` framework-only

Crate-private / sealed. With P1 or immediately after.

### P3 — full ascent + one `&mut Context` (no user posts yet)

Drop `Break`. Always return path. Thread `ctx` (depth 0, claim None). Exclusive via `ctx.claim()` try-take; must return path. Mercury `ascend_mut`+`set_layer` waits on reshape carrier — no `complete` token. Bind tests first if mercury blocks.

### P4 — `#[bind]` through `run_exclusive` only

No new attributes. Handlers return `(Vec<Effect>, P)`. Behavior-identical to P3.

### Feature steps (after P0–P4)

1. `#[post]`
2. `invalidate` / `step_up` / derived `validity()` (reshape may still be empty)
3. `exclusive` sugar naming settled (`exclusive` preferred over `if_unclaimed`)
4. `#[pre_post]`
5. mercury rearm; drop handle discriminant rearm
6. reshape carrier (open) — hop count `d` is chain length of `into_parent`s
7. generic `C` — `context-as-generic.md`

### Not prefactors

- Completion token
- Unused sink alone
- Root-owned reshape scheduler
- AndReturnHome restructure (needs a post)

## Rules

1. Descent schedules; that set is final. Generate: `opt_0`, `opt_1`, … only.
2. Ascent runs every scheduled post; mutation does not cancel them.
3. Pre: shared path. Post: owned path, return `(Vec<Effect>, P)`.
4. One `&mut Context` for the ascent. Posts take `&mut Context`.
5. `claim(&mut self) -> Option<Claimed>` try-takes. Not a getter. Not a parallel flag.
6. Context holds `depth: u32`. A kill's N× `into_parent` does `depth = depth.max(N)`. Framework ascent `step_up` decrements. Getter `validity(&self) -> Validity` is `depth == 0`.
7. Logging never calls `claim()`. Only exclusive does.
8. Every pre/post attr is a pre_post pair. Missing pre is well-known `noop_pre` (macro drops it in).
9. No `#[pre]` alone. A pre exists only as the first half of `#[pre_post]`.
10. User posts never take a dummy `()` to drop; `#[post]` bodies are `(node, ctx)`.
11. `#[bind]` = `(noop_pre, exclusive(h))` + event in the body.
12. Generate stays thin: schedule + call helpers. Bookkeeping is not expanded per node.
13. `empty_on_into_parent` is the empty `PathMut` `F` (no posts at that level).

## Tests

- scheduled post runs after deep bind, including when depth > 0 (`Invalidated`)
- logging pre_post never calls `claim()`
- deepest bind wins; parent bind skips after child `claim()`
- path threaded through two posts at one level
- pre return value consumed once
- pre miss: no post
- `#[post]` alone: expands to `(noop_pre, post)`; body is `(node, ctx)`
- `only_if_valid` / expression post
- depth: N× `into_parent` kill → `depth.max(N)`; N levels Invalidated then Valid above (after step_ups)
- `into_parent().into_parent()` → `depth.max(2)`

## Open

- Whether `pre` may also push now-effects on the way down.
- Reshape carrier: how a deep bind schedules a field replace at the owner; path return after today's `ascend_mut`+`set_layer` (hop count for `d` is settled: N = chain length).
- Sugar so user posts can write `-> Vec<Effect>` while the derive still threads path.
- Product nodes / multiple live children.
- Fallbacks that must not run if exclusive already took (`claim` already Some) — deferred; do not overload validity depth.
- Generic context type parameter — `context-as-generic.md`.
