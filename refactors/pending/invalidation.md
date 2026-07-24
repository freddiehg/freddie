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
    ctx: Context,
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
    ctx: Context,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    ctx: Context,
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

Leaf to root. The ascent threads one monotone `Claim` (`Open` at the leaf, `Taken` after an exclusive post runs). At each level the framework holds an owned path again.

1. Apply any reshape scheduled for this level's child field.
2. Build `Context` from the field's structure and the ascent's `Claim`.
3. For each scheduled post, call it with that `Context` and the owned path; take path and claim delta back from the return.
4. If the post reports `Claim::Taken`, the ascent claim becomes `Taken` for everything shallower.

A scheduled post always runs (is called). What it does with `ctx.claim()` is its business. Structure `Invalidated` is how it learns it lost the child.

```rust
#[derive(Clone, Copy)]
enum Structure {
    /// Child field is still the scheduled type. Projections through the path are sound.
    Valid,
    /// Child field was replaced.
    Invalidated,
}

/// Whether an exclusive has taken this event on the ascent so far.
#[derive(Clone, Copy)]
enum Claim {
    Open,
    Taken,
}

#[derive(Clone, Copy)]
struct Context {
    structure: Structure,
    claim: Claim,
}

impl Context {
    fn structure(self) -> Structure { self.structure }
    /// Snapshot: has an exclusive deeper in the tree already taken this event?
    fn claim(self) -> Claim { self.claim }
}
```

Access is flat on `Context`: `ctx.claim()`, `ctx.structure()`. No nested bag. Fields stay private so we can grow later without breaking call sites.

`claim()` is a snapshot at the moment this post runs. It is not "any pre/post below matched" — only exclusive posts raise `Taken`. A logging `AnyKey` pre/post never does.

### How `Claim` moves

```text
framework holds:  claim: Claim   (monotone Open → Taken)

each post call:
  ctx = Context { structure, claim }    // snapshot
  out = post(..., ctx)
  path = out.path
  claim = claim.join(out.claim)         // Taken wins; only exclusive sugar returns Taken
```

```rust
impl Claim {
    fn join(self, other: Claim) -> Claim {
        match (self, other) {
            (Claim::Taken, _) | (_, Claim::Taken) => Claim::Taken,
            _ => Claim::Open,
        }
    }
}

struct PostOut<P> {
    effects: Vec<Effect>,
    path: P,
    /// `Taken` if this post claims the event for exclusive deepest-wins; else `Open`.
    claim: Claim,
}
```

Plain posts always return `claim: Claim::Open`. `exclusive(h)` returns `Claim::Taken` only when it invoked `h`.

### Defaults

- `#[pre_post(trig => (pre, post))]` — user pre returns a value, user post receives it
- `#[pre(trig => pre)]` — user pre returns a value, post is `drop` of that value
- `#[post(trig => post)]` — pre is only the trigger check → `()`, user post receives `()`
- `#[bind(trig => handler)]` — a `#[post]` whose body is `exclusive(handler)`

Several pairs on one node: several independent `opt_i` (each its own concrete payload type), one `on_into_parent` closure. The derive names them by index (`opt_0`, `opt_1`, …), not by trigger or handler name — two attributes must not invent clashing identifiers.

### `#[bind]` is a post with no pre

There is no third handler kind and no secret claim flag outside `Context`.

```rust
#[bind(KeyA => outer_handler)]
// desugars to:
#[post(KeyA => exclusive(outer_handler))]
```

```rust
// user body — same signature as any post that needs the event:
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    ctx: Context,
) -> (Vec<M::Effect>, OuterPath);

// thin sugar — the only place that reads ctx.claim() for deepest-wins:
fn exclusive<E, P>(
    h: impl FnOnce(&E, Node<P, ()>, Context) -> (Vec<Effect>, P),
) -> impl FnOnce(&E, Node<P, ()>, Context) -> PostOut<P> {
    move |ev, node, ctx| {
        match ctx.claim() {
            Claim::Taken => PostOut {
                effects: Vec::new(),
                path: node.parent,
                claim: Claim::Open, // already Taken deeper; do not re-report
            },
            Claim::Open => {
                let (effects, path) = h(ev, node, ctx);
                PostOut {
                    effects,
                    path,
                    claim: Claim::Taken,
                }
            }
        }
    }
}
```

Framework when running any scheduled post:

```rust
let ctx = Context {
    structure, // Valid | Invalidated for this field
    claim,     // ascent claim so far
};
let out = post_fn(ev, Node { parent: path, data: () }, ctx);
Extend::extend(effs, out.effects);
path = out.path;
claim = claim.join(out.claim);
```

A plain `#[post]` is wrapped to `PostOut { claim: Claim::Open, ... }`. A `#[bind]` uses `exclusive`. pre_post posts at the same level are not exclusive: they all still run (before the bind at that level) and always see the same `ctx` snapshot unless an earlier post at this level returned Taken — order is pre_post posts first, then bind, so bind sees claims from below only, not from sibling pre_posts (sibling pre_posts do not claim).

### `only_if_valid`

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, Context) -> PostOut<P> {
    move |mut node, ctx| {
        let effects = match ctx.structure() {
            Structure::Valid => f(project(&mut node.parent)),
            Structure::Invalidated => Vec::new(),
        };
        PostOut {
            effects,
            path: node.parent,
            claim: Claim::Open,
        }
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

#[derive(Clone, Copy)]
pub enum Claim {
    Open,
    Taken,
}

impl Claim {
    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Taken, _) | (_, Self::Taken) => Self::Taken,
            _ => Self::Open,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Context {
    structure: Structure,
    claim: Claim,
}

impl Context {
    pub fn structure(self) -> Structure { self.structure }
    pub fn claim(self) -> Claim { self.claim }
}

pub struct PostOut<P> {
    pub effects: Vec<Effect>,
    pub path: P,
    pub claim: Claim,
}

pub struct PathMut<N, P, F> {
    // owned parent P + projections to N (as today)
    // F also receives claim so it can build Context for each post
    on_into_parent: F, // FnOnce(P, Structure, Claim) -> PostOut<P>
}

fn no_post<P>(parent: P, _: Structure, _: Claim) -> PostOut<P> {
    PostOut {
        effects: Vec::new(),
        path: parent,
        claim: Claim::Open,
    }
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, Structure, Claim) -> PostOut<P>,
{
    /// Apply reshape of the child field if any, run posts, return parent; join claim.
    pub fn into_parent(self, sink: &mut Vec<Effect>, claim: &mut Claim) -> P {
        let structure = self.classify_after_reshape();
        let out = (self.on_into_parent)(self.parent, structure, *claim);
        Extend::extend(sink, out.effects);
        *claim = claim.join(out.claim);
        out.path
    }
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim,
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
    let mut claim = Claim::Open;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    match claim {
        Claim::Taken => Some(effs),
        Claim::Open if !effs.is_empty() => Some(effs),
        Claim::Open => None,
    }
}
```

Handlers return effects via `PostOut`; only framework code holds the batch sink. `from_fn` is crate-private. The ascent's `Claim` is the same value snapshotted into every `Context`; posts never see a bare mutable flag.

## Generated code

### Helper the generate uses for every post

```rust
fn run_post<P>(
    claim: &mut Claim,
    structure: Structure,
    path: P,
    body: impl FnOnce(Node<P, ()>, Context) -> PostOut<P>,
) -> (P, Vec<Effect>) {
    let ctx = Context {
        structure,
        claim: *claim,
    };
    let out = body(
        Node {
            parent: path,
            data: (),
        },
        ctx,
    );
    *claim = claim.join(out.claim);
    (out.path, out.effects)
}
```

### Inner (leaf, one bind)

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim,
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
            let (path, out_effs) = run_post(
                claim,
                Structure::Valid, // leaf: no child field to invalidate
                path,
                |node, ctx| exclusive(inner_handler)(ev, node, ctx),
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
        claim: &mut Claim,
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

        // into_parent passes (structure, claim) into this closure.
        // opt_0 / opt_1 are captured; their types differ (Option of each pre's return).
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, structure, claim_now| {
                let mut claim = claim_now;
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(t0) = opt_0 {
                    let (p, e) = run_post(&mut claim, structure, path, |node, ctx| {
                        let (effects, path) = post_foo(t0, node, ctx);
                        PostOut {
                            effects,
                            path,
                            claim: Claim::Open,
                        }
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(t1) = opt_1 {
                    let (p, e) = run_post(&mut claim, structure, path, |node, ctx| {
                        let (effects, path) = post_bar(t1, node, ctx);
                        PostOut {
                            effects,
                            path,
                            claim: Claim::Open,
                        }
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                PostOut {
                    effects: local,
                    path,
                    claim,
                }
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claim);

        // ----- ascent: reshape + pre_post posts (Context built inside run_post) -----
        let mut path = inner_path.into_parent(effs, claim);

        // ----- bind opt_2: exclusive post -----
        if let ::core::option::Option::Some(()) = opt_2 {
            let structure = path.structure_of_inner();
            let ev = /* &KeyEvent from event */;
            let (p, e) = run_post(claim, structure, path, |node, ctx| {
                exclusive(outer_handler)(ev, node, ctx)
            });
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }
        path
    }
}
```

Every post goes through `run_post`. That is where `Context` is built (`structure` + current `claim`) and where `out.claim` updates the ascent bit. `exclusive` only reads `ctx.claim()` and sets `PostOut.claim`; it does not touch a parallel flag.

### `#[pre]` / `#[post]` alone

Same indexed opts. A bare `#[pre]` is `opt_i = Some(pre_return)` and the ascent arm is `drop(t_i)`. A bare `#[post]` is `opt_i = Some(())` and `run_post` with `claim: Claim::Open`.

## Walk

```text
DESCENT
  Outer owns path
  opt_0? = pre_foo return   (Foo pre_post)
  opt_1? = pre_bar return   (Bar pre_post)
  opt_2? = ()               (KeyA bind)
  move path into inner_path

ASCENT  claim starts Open
  Inner bind scheduled:
    ctx = Context { structure: Structure::Valid, claim: Claim::Open }
    exclusive(inner_handler) sees Claim::Open, runs body, PostOut.claim = Claim::Taken
    claim = Claim::Taken
  Outer into_parent:
    structure = Valid | Invalidated after reshape
    if opt_0: post_foo gets Context { structure, claim: Claim::Taken }
    if opt_1: post_bar same
  Outer bind (opt_2):
    ctx = Context { structure, claim: Claim::Taken }
    exclusive(outer_handler) sees Claim::Taken, skips body, claim: Claim::Open
```

### `KeyA` only

```text
Inner exclusive runs, claim Taken
Outer opt_2 exclusive sees Claim::Taken, skips
```

### `Foo` only

```text
opt_0 = Some(hits_before)
Inner: no bind
Outer post_foo(hits_before, path, ctx) with Claim::Open and Structure::Valid
opt_2 not scheduled
```

### `Foo` and `KeyA`

```text
Inner exclusive may reshape, claim Taken
Outer post_foo (opt_0) still runs — ctx.claim() is Claim::Taken
Outer exclusive (opt_2) skips
```

### Logging `AnyKey` pre_post also present

```text
also schedules, also runs with claim: Claim::Open on its PostOut
leaves claim at Claim::Open
does not change structure unless it mutates the child
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

Exclusive still takes path by value and may `ascend_mut`. No posts, no `Claim`.

### P1 — `into_parent` sink seam

```rust
// before
fn into_parent(self) -> Parent
// after
fn into_parent(self, _sink: &mut Vec<Effect>) -> Parent  // still just returns parent
```

Miss-unwind threads the sink. With no posts, untouched.

### P2 — `PathMut` carries `on_into_parent: F`, default `no_post`

`from_fn` takes a `FnOnce` that runs inside `into_parent`. All current sites pass `no_post` (returns path + empty effects). No user-facing posts yet. User code never constructs `F`.

### P3 — `from_fn` framework-only

`from_fn` / `from_box` crate-private (or sealed). Only the derive builds child paths.

### P4 — full ascent + `Claim` (still no user posts)

Drop `Break`. Every level returns its path. Thread `claim: &mut Claim`.

- child always returns path
- this level's exclusive runs only if claim is `Open`, then sets `Claim::Taken`

Deepest-wins without short-circuit past parents. Requires exclusives to **return the path**. Handlers that only `get_mut` adapt easily. Handlers that `ascend_mut` + `set_layer` wait on the reshape carrier (open) — do not invent `complete` to paper over it. If mercury blocks, ship P4 against bind tests first; mercury stays on Break until reshape.

### P5 — `PostOut` under existing `#[bind]` only

```rust
struct PostOut<P> {
    effects: Vec<Effect>,
    path: P,
    claim: Claim,
}
```

Generate rephrases `#[bind]` as a post-shaped call that sets `claim: Claim::Taken`. No new attributes. Behavior-identical to P4.

### Feature steps (after P0–P5)

1. `#[post(trig => body)]` — schedule `opt_N = Some(())`; run on ascent with owned path + context (claim from ascent; structure always Valid until step 2).
2. `Structure` Valid/Invalidated — classify in `into_parent` after reshape (reshape may still be empty).
3. `exclusive` + `#[bind]` as `#[post(exclusive(h))]` — claim via `Context` / `PostOut.claim`.
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
3. Pre: shared path. Post: owned path in/out via `PostOut`.
4. Every post receives `Context`; reads are `ctx.claim()`, `ctx.structure()`, `matches!(ctx.structure(), Structure::Valid)` (fields private).
5. Ascent holds a monotone `Claim`; each post sees a snapshot via `ctx.claim()`; exclusive sugar returns `Claim::Taken` when it runs the body; framework joins that into the ascent claim.
6. Logging always returns `Claim::Open`. Only `exclusive(...)` returns `Claim::Taken`.
7. no pre → `()`. no post → `drop`.
8. `#[bind]` = `#[post(exclusive(handler))]` — thin sugar over `ctx.claim()` / `PostOut.claim`.
9. Reshape of a field applied in that field's `into_parent` before posts at that level.

## Tests

- scheduled post runs after deep bind, including `Invalidated`
- logging `AnyKey` pre_post does not set `Claim::Taken`
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
- Fallbacks that must not run if claim is `Taken` (e.g. root `AnyKey` passthrough). Closer to `Claim` than to structure on `Context`. Deferred.
- Third structure state: deferred.
