# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root.

**Two types, both bind/freddie (not consumer types):**

- **`AscentState`** — the live ascent object. Constructed at the leaf turnaround. Fields private. Mutated only through methods: `invalidate`, `step_up`, `claim`. Threaded as `&mut AscentState` up the ascent.
- **`AscentStateSnapshot`** — a frozen view. Created by `state.snapshot()` at the start of each framework `into_parent` (before that level's posts). Posts read **`snap.mutation()`** only. They do not hold `&mut AscentState` and do not write the hop counter.

Nobody assigns `invalidation_depth` by hand. `invalidate(d)` and `step_up()` are real call sites in the framework / kill path (see below). `#[bind]` is a post with no pre, gated by `run_exclusive` → `state.claim()`.

**Generate stays thin.** Derive schedules `opt_N` and calls helpers. `snapshot` / `invalidate` / `step_up` / `claim` live in those helpers — not hand-rolled depth math in every expanded `Dispatch` impl.

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
#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]
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

// post: owned path; pre return; **snapshot only** (not &mut AscentState).
// Pre carriage exists so MaybeDropped still has the id after the child field is gone.
fn after_child(
    id: ChildId,
    node: Node<OuterPath, ()>,
    snap: &AscentStateSnapshot,
) -> (Vec<M::Effect>, OuterPath) {
    match snap.mutation() {
        Mutation::Intact => {
            let _ = (id, node.parent.get().inner.id);
            (vec![], node.parent)
        }
        Mutation::MaybeDropped => (vec![log_destroyed(id)], node.parent),
    }
}

// sugar: project + act only when snap says Intact
fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

// exclusive: event + node + **&mut AscentState** (may invalidate; claim is framework-side).
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, OuterPath) { ... }

// kill spine two hops up: each hop records invalidate; path recovery is separate
// from the framework into_parent that runs posts + step_up.
fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, InnerPath) {
    // example kill: climb 2 into_parents to the reshape owner
    let mut hops = 0u32;
    let path = node.parent;
    hops += 1;
    state.invalidate(hops); // call site: invalidate(1)
    let path = recover_parent(path); // path only; no step_up here
    hops += 1;
    state.invalidate(hops); // call site: invalidate(2) → depth.max(2)
    let path = recover_parent(path);
    // … set_layer / replace field at owner …
    (vec![], path)
}
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

Leaf to root.

1. **Leaf turnaround:** `let mut state = AscentState::new();` — not during descent.
2. **Exclusive at a level** (if scheduled): `run_exclusive` → `state.claim()`; body gets `&mut AscentState` and may **`state.invalidate(hops)`** on a kill climb (path recovery without `step_up`).
3. **Framework `into_parent`** (leave this level toward parent):

```rust
// laserbeam / bind — real body, not a comment
pub fn into_parent(self, sink: &mut Vec<Effect>, state: &mut AscentState) -> P {
    // 1. Freeze the view posts at THIS level will see
    let snap = state.snapshot(); // AscentStateSnapshot

    // 2. Run scheduled posts with &snap only (no &mut AscentState)
    let (parent, post_effs) = (self.on_into_parent)(self.parent, &snap);
    Extend::extend(sink, post_effs);

    // 3. Mutate the LIVE state after posts have read the snapshot
    state.step_up(); // call site: step_up

    parent
}
```

Order is fixed: **snapshot → posts read snapshot → `step_up` mutates live state**. Posts never see the post-`step_up` depth for their own level.

### `invalidate` call sites

Kill climb records hop count on the **live** `AscentState`. Not a field write.

```rust
// inside exclusive body (handler or helper)
let mut hops = 0u32;
hops += 1;
state.invalidate(hops); // depth = depth.max(1)
let path = recover_parent(path);
hops += 1;
state.invalidate(hops); // depth = depth.max(2)
let path = recover_parent(path);
```

`into_parent().into_parent()` as a kill climb ⇒ `invalidate(1)` then `invalidate(2)` ⇒ `depth.max(2)`. Concurrent kills still `max`. Path recovery during the kill does **not** call `step_up`; only framework `into_parent` does.

### `step_up` call sites

Only here:

```rust
// PathMut::into_parent, after posts
state.step_up();
```

Every framework hop leaf→root hits that line once.

### Snapshot vs live

| | `AscentState` | `AscentStateSnapshot` |
|---|---|---|
| when | leaf `new()`, then whole ascent | `state.snapshot()` at start of each framework `into_parent` |
| who mutates | `invalidate`, `step_up`, `claim` only | never (frozen) |
| who reads | framework; exclusive body | posts via `snap.mutation()` |
| claim | live, monotone try-take | not on the snapshot |

```rust
// crates/bind

#[derive(Clone, Copy)]
pub enum Mutation {
    /// No kill hop zone covers this level. Live depth was 0 at snapshot time.
    Intact,
    /// A deeper handler called `invalidate` after climbing through this level
    /// with path recovery (`into_parent` / `recover_parent`). Child fields along
    /// that climb may already be gone. Live depth was > 0 at snapshot time.
    MaybeDropped,
}

struct Claimed;

pub struct AscentState {
    invalidation_depth: u32,
    claim: Option<Claimed>,
}

/// Frozen at `snapshot()`. Posts take `&AscentStateSnapshot` only.
pub struct AscentStateSnapshot {
    mutation: Mutation,
}

impl AscentState {
    pub fn new() -> Self {
        Self {
            invalidation_depth: 0,
            claim: None,
        }
    }

    /// Freeze mutation view for posts at the current level.
    pub fn snapshot(&self) -> AscentStateSnapshot {
        AscentStateSnapshot {
            mutation: if self.invalidation_depth == 0 {
                Mutation::Intact
            } else {
                Mutation::MaybeDropped
            },
        }
    }

    /// Kill climb: `depth = depth.max(d)`. Call site in exclusive kill path.
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Framework `into_parent` only, after posts. Call site: see into_parent above.
    pub(crate) fn step_up(&mut self) {
        self.invalidation_depth = self.invalidation_depth.saturating_sub(1);
    }

    pub(crate) fn claim(&mut self) -> Option<Claimed> { /* try-take */ }

    pub(crate) fn claimed(&self) -> bool {
        self.claim.is_some()
    }
}

impl AscentStateSnapshot {
    pub fn mutation(&self) -> Mutation {
        self.mutation
    }
}
```

Handler-facing `pub`: `Mutation`, `AscentState::new`, `snapshot`, `invalidate`, `AscentStateSnapshot`, `AscentStateSnapshot::mutation`. Framework-only: `step_up`, `claim`, `claimed`, `Claimed`.

### How a two-hop kill walks

```text
DESCENT: schedule only — no AscentState

LEAF: state = AscentState::new()
  run_exclusive → claim()
  inner_handler:
    invalidate(1); recover_parent
    invalidate(2); recover_parent   // live depth == 2
    reshape at owner

FRAMEWORK into_parent (level +1, first hop up):
  snap = state.snapshot()           // MaybeDropped (depth 2)
  posts(&snap)                      // after_child, rearm see MaybeDropped
  state.step_up()                   // depth 2 → 1

FRAMEWORK into_parent (level +2):
  snap = state.snapshot()           // MaybeDropped (depth 1)
  posts(&snap)
  state.step_up()                   // depth 1 → 0

above:
  snap = state.snapshot()           // Intact (depth 0)
  …
```

Posts return `(Vec<Effect>, P)` only.

### Defaults

All attrs desugar to a pre_post pair. Missing pre is well-known `noop_pre` the macro drops in:

```rust
// in bind — not generated, not per-node
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}
```

| attr | expands to | descent | ascent |
|---|---|---|---|
| `#[pre_post(t => (pre, post))]` | `(pre, post)` | `opt = Some(pre(…))` | `post(t, node, &snap)` |
| `#[post(t => post)]` | `(noop_pre, post)` | `opt = Some(noop_pre(…))` | `post(node, &snap)` — **not** unit data |
| `#[bind(t => h)]` | `(noop_pre, exclusive(h))` | same as post | `run_exclusive` + `h(ev, node, &mut state)` |

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
    state: &mut AscentState,
) -> (Vec<M::Effect>, OuterPath);

fn run_exclusive<P>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match state.claim() {
        None => (path, Vec::new()),
        Some(Claimed) => body(Node { parent: path, data: () }, state),
    }
}
```

Call sites inside `on_into_parent` (posts already have `snap` from `into_parent`):

```rust
// snap taken in into_parent BEFORE this closure runs
if let Some(id) = opt_0 {
    let (path, effects) = run_post(path, &snap, |node, snap| after_child(id, node, snap));
    Extend::extend(local, effects);
}
if let Some(()) = opt_1 {
    let (path, effects) = run_post(path, &snap, |node, snap| {
        only_if_intact(|p| &mut p.get_mut().return_home, rearm)(node, snap)
    });
    Extend::extend(local, effects);
}
// after closure returns, into_parent calls state.step_up()
```

Sibling posts never call `claim()`. Order: posts by index, then bind (exclusive) after `into_parent` returns.

### `only_if_intact`

```rust
fn only_if_intact<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, &AscentStateSnapshot) -> (Vec<Effect>, P) {
    move |mut node, snap| {
        let effects = match snap.mutation() {
            Mutation::Intact => f(project(&mut node.parent)),
            Mutation::MaybeDropped => Vec::new(),
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

// Mutation, AscentState, AscentStateSnapshot, Claimed — full bodies in Semantics above.

pub struct PathMut<N, P, F> {
    // posts see snapshot only
    on_into_parent: F, // FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>)
}

fn empty_on_into_parent<P>(parent: P, _snap: &AscentStateSnapshot) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}

fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>),
{
    pub fn into_parent(self, sink: &mut Vec<Effect>, state: &mut AscentState) -> P {
        let snap = state.snapshot();
        let (parent, post_effs) = (self.on_into_parent)(self.parent, &snap);
        Extend::extend(sink, post_effs);
        state.step_up(); // explicit call site
        parent
    }
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> (Self::Path<'a>, AscentState)
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let (_path, state) = <N as Dispatch<M>>::dispatch(path, event, &mut effs);
    if state.claimed() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

## Generated code

### Helpers (in bind, not generated)

```rust
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

fn run_post<P>(
    path: P,
    snap: &AscentStateSnapshot,
    body: impl FnOnce(Node<P, ()>, &AscentStateSnapshot) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    body(Node { parent: path, data: () }, snap)
}

fn run_exclusive<P>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match state.claim() {
        None => (path, Vec::new()),
        Some(Claimed) => body(Node { parent: path, data: () }, state),
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
    ) -> (<Inner as Place>::Path<'a>, AscentState)
    where
        Self: 'a,
    {
        // ----- descent: schedule only; no AscentState -----
        // index 0: #[bind(KeyA => inner_handler)]
        let opt_0 = if let ::core::option::Option::Some(ev) =
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

        // ----- ascent begins: construct AscentState -----
        let mut state = AscentState::new();

        if let ::core::option::Option::Some(()) = opt_0 {
            let ev = /* &KeyEvent from event */;
            // body may state.invalidate(hops) on a kill climb
            let (path, out_effs) = run_exclusive(path, &mut state, |node, state| {
                inner_handler(ev, node, state)
            });
            ::core::iter::Extend::extend(effs, out_effs);
            return (path, state);
        }
        (path, state)
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
    ) -> (<Outer as Place>::Path<'a>, AscentState)
    where
        Self: 'a,
    {
        // ----- descent: schedule only; no AscentState -----
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

        // opt_1: #[post(AnyKey => only_if_intact(..., rearm))] via noop_pre
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
            // snap is created in into_parent BEFORE this runs; step_up AFTER
            move |parent, snap| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = run_post(path, snap, |node, snap| after_child(id, node, snap));
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(()) = opt_1 {
                    let (p, e) = run_post(path, snap, |node, snap| {
                        only_if_intact(|p| &mut p.get_mut().return_home, rearm)(node, snap)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                (path, local)
            },
        );

        // child: leaf constructs AscentState; may invalidate on kill
        let (inner_path, mut state) =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs);

        // into_parent: snap = state.snapshot(); posts(&snap); state.step_up();
        let mut path = inner_path.into_parent(effs, &mut state);

        if let ::core::option::Option::Some(()) = opt_2 {
            let ev = /* &KeyEvent from event */;
            let (p, e) = run_exclusive(path, &mut state, |node, state| {
                outer_handler(ev, node, state)
            });
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }
        (path, state)
    }
}
```

### `#[post]` alone

Same indexed opts. Expands to `(noop_pre, post)`. User body is `(node, &snap)`. Never `claim()`.

## Walk

```text
DESCENT  (no AscentState)
  opt_0? opt_1? opt_2?
  move path into child

ASCENT
  leaf: state = AscentState::new()
  Inner exclusive: claim(); body may invalidate(1), invalidate(2), …
  Outer into_parent:
    snap = state.snapshot()
    after_child(&snap); rearm via only_if_intact(&snap)
    state.step_up()
  Outer exclusive: claim() — fails if Inner took it
```

### `KeyA` (AnyKey pre_post + post + KeyA bind; Inner kills 2 hops)

```text
DESCENT: snap_child_id; schedule rearm; schedule outer bind
Inner claim() ok
  invalidate(1); recover_parent
  invalidate(2); recover_parent     // live depth 2
Outer into_parent:
  snapshot → MaybeDropped
  after_child: log_destroyed(id)
  only_if_intact rearm: skip; Drop cancels guard
  step_up → depth 1
  (further step_ups until depth 0 above owner)
Outer exclusive: claim already taken → skip
```

### `KeyB` (AnyKey only; no bind)

```text
no invalidate
Outer into_parent: snapshot → Intact
after_child: child live
rearm: new guard + schedule
step_up
never claim()
```

## Rearm

`#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]` — see DX above.

`snap.mutation() == Intact`: rearm. `MaybeDropped`: skip; `Drop` of the guard cancels. Does not call `claim`.

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

### P3 — full ascent + `AscentState` (no user posts yet)

Drop `Break`. Always return path. Leaf `AscentState::new()`; return `(path, state)`. Exclusive via `state.claim()`. `into_parent` already: `snapshot` + empty posts + `step_up` (even with no user posts). Mercury kill path later wires `invalidate`. Bind tests first if mercury blocks.

### P4 — `#[bind]` through `run_exclusive` only

No new attributes. Handlers return `(Vec<Effect>, P)`. Behavior-identical to P3.

### Feature steps (after P0–P4)

1. `#[post]` reading `AscentStateSnapshot`
2. kill path: explicit `invalidate(hops)` call sites (reshape carrier may still be open)
3. `exclusive` sugar naming settled (`exclusive` preferred over `if_unclaimed`)
4. `#[pre_post]`
5. mercury rearm; drop handle discriminant rearm
6. reshape carrier (open) — hop count `d` is kill-climb length; each hop `invalidate(hops)`

### Not prefactors

- Completion token
- Unused sink alone
- Root-owned reshape scheduler
- AndReturnHome restructure (needs a post)

## Rules

1. Descent schedules; that set is final. Generate: `opt_0`, `opt_1`, … only.
2. Ascent runs every scheduled post; path mutation does not cancel them.
3. Pre: shared path. Post: owned path, return `(Vec<Effect>, P)`.
4. Live `AscentState` + frozen `AscentStateSnapshot`. Both bind/freddie. No direct field writes.
5. Framework `into_parent`: `let snap = state.snapshot();` → posts(`&snap`) → `state.step_up()`.
6. Kill climb: explicit `state.invalidate(hops)` each hop; path recovery without `step_up`.
7. Posts take `&AscentStateSnapshot` only. Exclusive takes `&mut AscentState`.
8. Handler-facing `pub`: `Mutation`, `AscentState::{new, snapshot, invalidate}`, `AscentStateSnapshot::mutation`. Framework-only: `step_up`, `claim`, `claimed`, `Claimed`.
9. Logging never calls `claim()`. Only `run_exclusive` does.
10. Every pre/post attr is a pre_post pair. Missing pre is `noop_pre`.
11. No `#[pre]` alone.
12. `#[post]` bodies are `(node, &snap)` — no unit data arg.
13. `#[bind]` = `(noop_pre, exclusive(h))` + event + `&mut AscentState`.
14. Generate stays thin: schedule + call helpers.
15. `empty_on_into_parent` is the empty `PathMut` `F`.

## Tests

- scheduled post runs after deep bind; sees `MaybeDropped` via snapshot when kill set depth > 0
- logging pre_post never calls `claim()`
- deepest bind wins; parent bind skips after child `claim()`
- path threaded through two posts at one level
- pre return value consumed once
- pre miss: no post
- `#[post]` alone: `(node, &snap)`
- `only_if_intact` / expression post
- kill: `invalidate(1)` then `invalidate(2)` → depth.max(2); each framework `into_parent` calls `step_up`
- posts at a level see snapshot taken before that level's `step_up`, not after

## Open

- Whether `pre` may also push now-effects on the way down.
- Reshape carrier: how a deep bind schedules a field replace at the owner; path return after today's `ascend_mut`+`set_layer` (hop count for `d` is settled: N = chain length).
- Sugar so user posts can write `-> Vec<Effect>` while the derive still threads path.
- Product nodes / multiple live children.
- Fallbacks that must not run if exclusive already took (`claim` already Some) — deferred; do not overload invalidation_depth.
