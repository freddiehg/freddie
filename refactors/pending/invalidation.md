# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root; mutation is free; each post is handed a `Context` — structure of the child field plus whether an exclusive already claim this event (a small grab bag, not a pure "validity" flag). `#[bind]` is a post that reads the claim bit on that context.

## Paths: shared down, owned up

The path type is the normal owned `PathMut` (same as today). What changes is when the handler may hold it.

- **Descent (pre):** the framework still owns the path for the walk. Pre gets a shared borrow: `Node<&P, D>`. Read-only; no reshape.
- **Ascent (post / bind):** `into_parent` has recovered this level's path as an owned `P` again (built the usual way: `from_fn` on the way down, `into_parent` on the way up). Post and bind receive that owned path: `Node<P, D>` — same shape as today's exclusive handlers. `get_mut`, `ascend_mut`, the usual tools.

There is no parallel "`&mut Path`" API for posts. Mutation is allowed because ownership of the normal path is back at this level on the way up, not because the signature is a mutable reference.

To thread ownership through several posts at one level, each post/bind returns the path with its effects. The value carried from pre to post is whatever pre returns — a concrete type, inferred, not a type parameter of the framework.

## Developer experience

```rust
#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[pre(Baz => track)]                 // post is drop
#[post(Qux => guard)]                // pre is trigger → ()
#[bind(KeyA => outer_handler)]       // one function; exclusive
struct Outer {
    #[resolve_into]
    inner: Inner,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner;
```

```rust
// pre: shared path. Return type is ordinary and concrete (here u32); the
// framework stores it in Option<_> inferred from this function.
fn pre_foo(ev: &FooEvent, node: Node<&OuterPath, ()>) -> u32 {
    node.parent.get().hits
}

// post: owned path in, path out; first arg is pre_foo's return (u32);
// Context carries structure + whether an exclusive has already taken the event.
fn post_foo(
    hits_before: u32,
    node: Node<OuterPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, OuterPath) {
    match ctx.structure() {
        Structure::Valid => {
            // node.parent.get_mut().inner is still Inner
            let _ = (hits_before, ctx.claim());
            (vec![], node.parent)
        }
        Structure::Invalidated => (vec![], node.parent),
    }
}

// pre_bar / post_bar likewise — whatever pre_bar returns is what post_bar receives.
// Often that is () because the pair only needs timing, not carriage.

// bind body: ordinary post shape. exclusive() only gates on ctx.claim().
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

// sugar: act only when the child survived (no pre carriage)
fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}
// #[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
```

Expression positions work as today (`#handler(…)` splice). Pinned by `crates/bind/tests/expr_handler.rs`.

## Semantics

### Schedule on the way down (final)

For each pre/post/bind on the node, if the trigger matches:

- `#[pre_post]` / `#[pre]`: call pre with `Node<&P, D>`, store `opt_N = Some(pre_return)`
- `#[post]`: store `opt_N = Some(())`
- `#[bind]`: store `opt_N = Some(())` (same as `#[post]`; event stays the dispatch `&Event`)

`N` is the attribute index on the node (0, 1, 2, …). The derive always emits `opt_0`, `opt_1`, … — never names from triggers or handlers. If the trigger misses, `opt_N = None`. Ascent never re-checks triggers and never changes which opts are `Some`.

```rust
// shape only — return type is whatever the pre function returns (concrete, inferred)
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> /* concrete type */
```

No reshape on the descent. Whether pre may also return now-effects is open; generate is return-value only (no effect batch from pre).

### Execute on the way up (all scheduled posts run)

Leaf to root. The ascent threads one **`&mut Context`**. There is no parallel claim bit beside it — claim lives on the context and is mutated in place.

1. Apply any reshape scheduled for this level's child field.
2. `ctx.set_structure(...)` for that field (Valid / Invalidated).
3. For each scheduled post, call it with owned path and `&mut ctx`; take path back from the return.
4. Exclusive posts mutate `ctx` when they take the event (`ctx.set_claim(Claimed)`).

A scheduled post always runs (is called). What it does with `ctx.claim()` / `ctx.structure()` is its business.

```rust
#[derive(Clone, Copy)]
enum Structure {
    /// Child field is still the scheduled type. Projections through the path are sound.
    Valid,
    /// Child field was replaced.
    Invalidated,
}

/// ZST marker: an exclusive has taken this event.
#[derive(Clone, Copy)]
struct Claimed;

struct Context {
    structure: Structure,
    claim: Option<Claimed>,
}

impl Context {
    fn structure(&self) -> Structure { self.structure }
    /// `None` = open; `Some(Claimed)` = an exclusive already took this event.
    fn claim(&self) -> Option<Claimed> { self.claim }
    fn set_structure(&mut self, s: Structure) { self.structure = s; }
    fn set_claim(&mut self, c: Claimed) { self.claim = Some(c); }
}
```

Access is flat on `Context`. Fields private. Posts that only read use `&Context` if we want; the ascent always has `&mut Context` so exclusive can write claim and `into_parent` can write structure.

`claim()` is not "any pre/post below matched" — only exclusive posts call `set_claim`. A logging `AnyKey` pre/post never does.

### How claim moves

```text
framework holds:  ctx: Context   (one value for the whole ascent)

at each level:
  ctx.set_structure(Valid | Invalidated)   // for this field after reshape

each plain post:
  (effects, path) = post(..., &mut ctx)    // may read; must not need to set claim

each exclusive (#[bind]):
  match ctx.claim() {
    Some(_) => skip body
    None => {
      (effects, path) = h(..., &mut ctx)
      ctx.set_claim(Claimed)
    }
  }
```

Posts return `(Vec<Effect>, P)` — path threading only. Claim and structure are mutated on `Context`, not returned.

### Defaults

- `#[pre_post(trig => (pre, post))]` — user pre returns a value, user post receives it
- `#[pre(trig => pre)]` — user pre returns a value, post is `drop` of that value
- `#[post(trig => post)]` — pre is only the trigger check → `()`, user post receives `()`
- `#[bind(trig => handler)]` — a `#[post]` whose body is `exclusive(handler)`

Several pairs on one node: several independent `opt_i` (each its own concrete payload type), one `on_into_parent` closure. The derive names them by index (`opt_0`, `opt_1`, …), not by trigger or handler name — two attributes must not invent clashing identifiers.

### `#[bind]` is a post with no pre

There is no third handler kind and no claim channel outside `Context`. Mutate the context.

```rust
#[bind(KeyA => outer_handler)]
// desugars to:
#[post(KeyA => exclusive(outer_handler))]
```

```rust
// user body — &mut Context (may read structure/claim; exclusive writes claim):
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    ctx: &mut Context,
) -> (Vec<M::Effect>, OuterPath);

// call site (or thin exclusive helper) — all mutation of claim goes through ctx:
fn run_exclusive<E, P>(
    ev: &E,
    path: P,
    ctx: &mut Context,
    h: impl FnOnce(&E, Node<P, ()>, &mut Context) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match ctx.claim() {
        Some(_) => (path, Vec::new()),
        None => {
            let (effects, path) = h(ev, Node { parent: path, data: () }, ctx);
            ctx.set_claim(Claimed);
            (path, effects)
        }
    }
}
```

Plain post call site:

```rust
let (effects, path) = post_fn(ev, Node { parent: path, data: () }, ctx);
Extend::extend(effs, effects);
```

pre_post posts at the same level are not exclusive: they all still run (before the bind). Sibling pre_posts never call `set_claim`.

### `only_if_valid`

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, &mut Context) -> (Vec<Effect>, P) {
    move |mut node, ctx| {
        let effects = match ctx.structure() {
            Structure::Valid => f(project(&mut node.parent)),
            Structure::Invalidated => Vec::new(),
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
pub enum Structure {
    Valid,
    Invalidated,
}

/// ZST marker: an exclusive took this event.
pub struct Claimed;

pub struct Context {
    structure: Structure,
    claim: Option<Claimed>,
}

impl Context {
    pub fn structure(&self) -> Structure { self.structure }
    /// `None` = open; `Some(Claimed)` = exclusive already took this event.
    pub fn claim(&self) -> Option<Claimed> { self.claim }
    pub fn set_structure(&mut self, s: Structure) { self.structure = s; }
    pub fn set_claim(&mut self, c: Claimed) { self.claim = Some(c); }
}

pub struct PathMut<N, P, F> {
    // owned parent P + projections to N (as today)
    on_into_parent: F, // FnOnce(P, &mut Context) -> (P, Vec<Effect>)
}

fn no_post<P>(parent: P, _ctx: &mut Context) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, &mut Context) -> (P, Vec<Effect>),
{
    /// Apply reshape, set structure on ctx, run posts, return parent.
    pub fn into_parent(self, sink: &mut Vec<Effect>, ctx: &mut Context) -> P {
        ctx.set_structure(self.classify_after_reshape());
        let (parent, post_effs) = (self.on_into_parent)(self.parent, ctx);
        Extend::extend(sink, post_effs);
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
        structure: Structure::Valid,
        claim: None,
    };
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut ctx);
    if ctx.claim().is_some() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

Handlers return `(Vec<Effect>, P)`; only framework code holds the batch sink. `from_fn` is crate-private. One `Context` is mutated for the whole ascent.

## Generated code

### Helpers

```rust
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
        Some(_) => (path, Vec::new()),
        None => {
            let (effects, path) = body(Node { parent: path, data: () }, ctx);
            ctx.set_claim(Claimed);
            (path, effects)
        }
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
        // index 0 on this node: #[bind(KeyA => inner_handler)]
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
            ctx.set_structure(Structure::Valid); // leaf
            let (path, out_effs) = run_exclusive(
                path,
                ctx,
                |node, ctx| inner_handler(ev, node, ctx),
            );
            ::core::iter::Extend::extend(effs, out_effs);
            return path;
        }
        path
    }
}
```

### Outer (two pre_posts + one bind)

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
        // ----- descent: schedule -----
        // Attribute order on Outer is the index:
        //   0  #[pre_post(Foo => (pre_foo, post_foo))]
        //   1  #[pre_post(Bar => (pre_bar, post_bar))]
        //   2  #[bind(KeyA => outer_handler)]
        let opt_0 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = Foo;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(pre_foo(
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

        let opt_1 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = Bar;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(pre_bar(
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

        let opt_2 = if let ::core::option::Option::Some(ev) =
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

        // into_parent passes &mut Context (structure already set for this field).
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, ctx| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(t0) = opt_0 {
                    let (p, e) = run_post(path, ctx, |node, ctx| post_foo(t0, node, ctx));
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(t1) = opt_1 {
                    let (p, e) = run_post(path, ctx, |node, ctx| post_bar(t1, node, ctx));
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                (path, local)
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, ctx);

        // ----- ascent: reshape + set_structure + pre_post posts -----
        let mut path = inner_path.into_parent(effs, ctx);

        // ----- bind opt_2: exclusive post -----
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

Plain posts go through `run_post`; binds go through `run_exclusive`. Both take `&mut Context`. Only `run_exclusive` calls `ctx.set_claim(Claimed)`.

### `#[pre]` / `#[post]` alone

Same indexed opts. A bare `#[pre]` is `opt_i = Some(pre_return)` and the ascent arm is `drop(t_i)`. A bare `#[post]` is `opt_i = Some(())` and `run_post` (does not write claim).

## Walk

```text
DESCENT
  Outer owns path
  opt_0? opt_1? opt_2?
  move path into inner_path

ASCENT  one &mut ctx (structure Valid, claim None)
  Inner bind:
    ctx.set_structure(Valid)
    run_exclusive: claim None → body → ctx.set_claim(Claimed)
  Outer into_parent:
    ctx.set_structure(Valid|Invalidated)
    post_foo / post_bar get &mut ctx (claim already Some)
  Outer bind:
    run_exclusive: claim Some → skip body
```

### `KeyA` only

```text
Inner set_claim; Outer exclusive sees Some and skips
```

### `Foo` only

```text
post_foo runs with claim None; no exclusive
```

### `Foo` and `KeyA`

```text
Inner exclusive may reshape and set_claim
Outer post_foo still runs (reads ctx)
Outer exclusive skips
```

### Logging `AnyKey` pre_post also present

```text
runs; never set_claim
```

## Rearm

```rust
#[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`structure: Valid`: rearm. `Invalidated`: skip; `Drop` of the guard cancels the timer. Does not care about `claim`.

## Prefactors (ordered, each shippable alone)

Behavior-identical to master until a step says otherwise. No `#[pre]` / `#[post]` until the feature steps. No completion token.

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

Today: `type Output`, handler return collected into `Break(out)`.

After:

- `type Effect` is the **item** (`MercuryEffect`), not the vec
- `dispatch(path, event, effs: &mut Vec<Effect>) -> ControlFlow<(), Path>`
- exclusive pushes onto `effs` and `Break(())`; miss is `Continue(path)`
- top-level seeds `Vec`, `Some(effs)` on win, `None` on total miss
- handler return `V: Into<Vec<Effect>>` (single effect or vec)
- mercury: `From<MercuryEffect> for Vec<MercuryEffect>`

Exclusive still takes path by value and may `ascend_mut`. No posts, no claim tracking.

### P1 — `on_into_parent` + sink (together, not sink alone)

Do not add an unused `sink` argument "for later." The sink appears in the same change that runs a post and pushes into it.

```rust
// before
fn into_parent(self) -> Parent

// after — one step
PathMut { …, on_into_parent: F }
fn into_parent(self, sink: &mut Vec<Effect>) -> Parent {
    let (parent, effs) = (self.on_into_parent)(self.parent /*, … */);
    sink.extend(effs);
    parent
}
```

All current sites pass `no_post` (returns parent + empty effects). Behavior-identical. User code never constructs `F`.

### P2 — `from_fn` framework-only

`from_fn` / `from_box` crate-private (or sealed). Only the derive builds child paths. Can ship with P1 or immediately after.

### P3 — full ascent + one `&mut Context` (still no user posts)

Drop `Break`. Every level returns its path. Thread `ctx: &mut Context` (starts structure Valid, claim None).

- child always returns path
- exclusive runs only if `ctx.claim()` is `None`, then `ctx.set_claim(Claimed)`

Deepest-wins without short-circuit past parents. Requires exclusives to **return the path**. Handlers that only `get_mut` adapt easily. Handlers that `ascend_mut` + `set_layer` wait on the reshape carrier (open). If mercury blocks, ship P3 against bind tests first.

### P4 — exclusive call shape under existing `#[bind]` only

Generate rephrases `#[bind]` through `run_exclusive` / `if_unclaimed` (read/write claim on `&mut Context`). Handlers return `(Vec<Effect>, P)`. No new attributes. Behavior-identical to P3. Naming: `exclusive` or `if_unclaimed` — both fine; `exclusive` names the role, `if_unclaimed` names the gate (and does not by itself imply mutation — `set_claim` does).

### Feature steps (after P0–P4)

1. `#[post(trig => body)]` — schedule `opt_N = Some(())`; run on ascent with owned path + `&mut Context`.
2. `Structure` Valid/Invalidated — `ctx.set_structure` in `into_parent` after reshape.
3. `exclusive` + `#[bind]` as sugar — mutates `ctx` claim at the call site.
4. `#[pre]` / `#[pre_post]` — carriage; immutable path on descent.
5. mercury rearm as post; drop `handle` discriminant rearm; timed-layer wrapper when ready.
6. reshape carrier (open) — deep bind schedules field replace at owner.
7. generic `C` — `context-as-generic.md`.

### Not prefactors

- Completion token / gather-on-climb
- Root-owned reshape scheduler
- AndReturnHome tree move (needs a post for rearm)

## Rules

1. Descent schedules; that set is final.
2. Ascent runs every scheduled post; mutation does not cancel them.
3. Pre: shared path. Post: owned path in/out as `(Vec<Effect>, P)`.
4. Ascent threads one `&mut Context`. Posts take `&mut Context`. No parallel claim flag.
5. `ctx.claim() -> Option<Claimed>`, `ctx.structure()`, `set_structure` / `set_claim` (fields private).
6. Logging never calls `set_claim`. Only exclusive (`#[bind]`) does.
7. no pre → `()`. no post → `drop`.
8. `#[bind]` = `#[post]` + `run_exclusive` (read/write claim on the same context).
9. Reshape of a field applied in that field's `into_parent` before posts at that level (`set_structure` first).

## Tests

- scheduled post runs after deep bind, including `Invalidated`
- logging `AnyKey` pre_post does not set `Some(Claimed)`
- deepest bind wins; parent bind runs when leaf misses
- path is returned through two posts at one level
- drop counter on a pre return value: consumed once by post (or drop)
- pre miss: no post
- `#[pre]` alone: drop; `#[post]` alone: receives `()`
- expression post: `only_if_valid(project, rearm)`

## Open

- Whether `pre` may also push now-effects on the way down.
- How a deep bind schedules a reshape of a field it does not own (carrier applied in owner's `into_parent`). Until then, binds that `ascend_mut` + `set_layer` and path return do not compose cleanly.
- Sugar so user posts can write `-> Vec<Effect>` while the derive still threads path.
- Product nodes: one Context (structure bit) per live child field.
- Fallbacks that must not run if claim is `Some` (e.g. root `AnyKey` passthrough). Closer to `Option<Claimed>` than to structure on `Context`. Deferred.
- Third structure state: deferred.
