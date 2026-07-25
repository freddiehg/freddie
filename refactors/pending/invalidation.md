# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root.

## What this is solving

A deep exclusive may destroy a child field (reshape / set_layer). Ancestor posts scheduled for the same event must still run. They need to know whether the spine under them was destroyed, without re-checking triggers and without reading a field that may no longer exist.

That knowledge is a hop counter on a live ascent object, frozen into a snapshot for the posts at each level.

## Two objects (bind / freddie)

- **`AscentState`** — **internal** to the dispatch machine (`pub(crate)` in `bind`). Live hop counter + claim. Constructed at leaf turnaround. Threaded only through framework code (`dispatch`, `into_parent`, `run_exclusive`). User posts never see it. Private fields; mutate only via methods.
- **`AscentStateSnapshot`** — **what is passed into user post functions**. Frozen at `state.snapshot()` when framework `into_parent` starts, before that level's posts. Posts take `&AscentStateSnapshot` and call `snap.mutation()`.

Exclusive bodies are framework-gated (`run_exclusive`); they may hold `&mut AscentState` only inside that gate so they can `invalidate` / participate in `claim`. That is not the post path. Posts get the snapshot only.

No hand assignment of `invalidation_depth`.

## Paths: shared down, owned up

The path type is the normal owned `PathMut` (same as today).

- **Descent (pre):** framework owns the path. Pre gets `Node<&P, D>`. Read-only; no reshape.
- **Ascent (post / bind):** at this level, the path is owned `P` again. Post and bind receive `Node<P, D>`.

The framework path stack is **not** consumed by a kill. A kill records `invalidate(N)` and (separately, open) schedules reshape at the owner. Path recovery for reshape is the reshape carrier — not a second parallel `into_parent` inventing a different path type. Exclusive handlers return the **same level's** `P` they were given. Framework `into_parent` still runs hop by hop and still runs posts.

## Order at a level (non-leaf)

```text
1. descent already scheduled opt_N
2. from_fn into child; child dispatch returns (child_path, state)
3. framework into_parent(child_path, &mut state):
     snap = state.snapshot()
     run this level's posts with &snap
     state.step_up()
4. this level's exclusive (if scheduled): run_exclusive → claim; body gets &mut state
```

Leaf has no child: construct `state`, run exclusive if any, return `(path, state)`.

## `into_parent` — snapshot, posts, step_up

```rust
// laserbeam / bind
pub fn into_parent(self, sink: &mut Vec<Effect>, state: &mut AscentState) -> P {
    let snap = state.snapshot();
    let (parent, post_effs) = (self.on_into_parent)(self.parent, &snap);
    Extend::extend(sink, post_effs);
    state.step_up();
    parent
}
```

Call sites:

- `state.snapshot()` — once per framework hop, before posts
- `state.step_up()` — once per framework hop, after posts

Posts never see the post-`step_up` depth for their own level.

`on_into_parent: FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>)`.

## `invalidate` — kill records hop depth on the live state

A kill that will destroy a spine of height `N` (N framework hops from the exclusive's level up to and including the reshape owner) calls:

```rust
state.invalidate(N); // depth = depth.max(N)
```

That is the call site. One call with the full height, not a hand-rolled loop of fake `recover_parent`s that steal the framework path.

```rust
// exclusive body — still at InnerPath
fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, InnerPath) {
    // Reshape owner is two hops above this exclusive (Outer then Root, or
    // Outer as owner with N=1 — example uses N=2).
    state.invalidate(2);
    // schedule reshape at owner (carrier open) — do not climb PathMut here
    (vec![/* reshape effect / carrier */], node.parent)
}
```

`invalidate` only moves the counter. It does not recover path. Framework `into_parent` still walks the real stack.

Concurrent kills: `depth = depth.max(d)` so a deeper kill is not shrunk.

## Why a snapshot type (not `&AscentState` for posts)

Posts must not call `invalidate` / `step_up` / `claim`. A separate snapshot type makes that unrepresentable: posts only have `mutation()`. The freeze happens at the one moment that defines the level — entry to `into_parent` — before `step_up`.

## Types

```rust
#[derive(Clone, Copy)]
pub enum Mutation {
    /// Live depth was 0 when this snapshot was taken.
    Intact,
    /// Live depth was > 0 when this snapshot was taken: a deeper exclusive
    /// called `invalidate(N)` covering this hop. Child fields in that zone
    /// may already be gone (or will be when reshape runs). Prefer pre carriage
    /// over reading the child field.
    MaybeDropped,
}

struct Claimed; // crate-private; only claim() produces it

/// Internal to bind dispatch. Not passed to user posts.
pub(crate) struct AscentState {
    invalidation_depth: u32,
    claim: Option<Claimed>,
}

/// Passed to user post functions.
pub struct AscentStateSnapshot {
    mutation: Mutation,
}

impl AscentState {
    pub(crate) fn new() -> Self {
        Self {
            invalidation_depth: 0,
            claim: None,
        }
    }

    pub(crate) fn snapshot(&self) -> AscentStateSnapshot {
        AscentStateSnapshot {
            mutation: if self.invalidation_depth == 0 {
                Mutation::Intact
            } else {
                Mutation::MaybeDropped
            },
        }
    }

    /// Kill: depth = depth.max(d). Exclusive / kill helper only.
    pub(crate) fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Framework into_parent only, after posts.
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

Visibility:

- `pub` (user-facing): `Mutation`, `AscentStateSnapshot`, `AscentStateSnapshot::mutation`
- `pub(crate)` (internal): whole `AscentState`, `new` / `snapshot` / `invalidate` / `step_up` / `claim` / `claimed`, `Claimed`
- Exclusive user fns in other crates: still need a public way to `invalidate` — either a thin public wrapper type or kill helpers in bind. Open until reshape carrier lands; do not pass raw internal state to posts.

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
fn snap_child_id(ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

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

fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}

fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, InnerPath) {
    state.invalidate(2);
    // reshape carrier (open) schedules field replace at owner
    (vec![], node.parent)
}
```

Signatures:

| kind | args | path return |
|---|---|---|
| pre | `(ev, Node<&P,D>) -> T` | n/a |
| post | `(T, Node<P,D>, &AscentStateSnapshot) -> (Vec<Effect>, P)` or without T if noop_pre | same level `P` |
| exclusive | `(ev, Node<P,D>, &mut AscentState) -> (Vec<Effect>, P)` | same level `P` |

`#[bind]` is **not** the same signature as `#[post]`. Desugar: schedule like post (`noop_pre` token) + run through `run_exclusive` (claim + `&mut AscentState`). Do not pretend the body is a post that takes a snapshot.

## Schedule (descent)

Every pre/post attr is a pre_post pair:

- `#[pre_post(t => (pre, post))]` → `(pre, post)`
- `#[post(t => post)]` → `(noop_pre, post)`
- `#[bind(t => h)]` → schedule token like post; ascent runs `run_exclusive(h)`

Trigger match → `opt_N = Some(pre_return)`; miss → `None`. Ascent never re-checks triggers. Index `opt_0`, `opt_1`, … only.

## Defaults / helpers

```rust
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

## Walk: Root → Outer → Inner, KeyA, Inner kills with N=2

Tree: Root contains Outer contains Inner. Exclusive on Inner sets `invalidate(2)`.

```text
DESCENT Outer: schedule after_child, rearm post, outer bind
DESCENT Inner: schedule inner bind
  (no AscentState yet)

LEAF Inner:
  state = AscentState::new()
  run_exclusive → claim() ok
  inner_handler: state.invalidate(2)   // live depth = 2
  return (inner_path, state)           // still InnerPath

Outer into_parent (the ONE hop Inner → Outer):
  snap = state.snapshot()              // MaybeDropped (depth 2)
  after_child(&snap)                   // pre id; do not trust .inner
  only_if_intact rearm → skip; Drop cancels guard
  state.step_up()                      // depth 2 → 1

Outer exclusive:
  claim() fails (Inner took it) → skip

Root into_parent (Outer → Root), if Root has posts:
  snap = state.snapshot()              // MaybeDropped (depth 1)
  posts…
  state.step_up()                      // depth 1 → 0

above: Intact
```

KeyB (AnyKey posts only, no bind): no `invalidate`; every snapshot Intact; rearm runs; never `claim`.

## Types (dispatch)

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub struct PathMut<N, P, F> {
    on_into_parent: F, // FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>)
}

fn empty_on_into_parent<P>(parent: P, _snap: &AscentStateSnapshot) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>),
{
    pub fn into_parent(self, sink: &mut Vec<Effect>, state: &mut AscentState) -> P {
        let snap = state.snapshot();
        let (parent, post_effs) = (self.on_into_parent)(self.parent, &snap);
        Extend::extend(sink, post_effs);
        state.step_up();
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
        // ----- descent: schedule only; no AscentState yet -----
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

        // ----- ascent begins: construct internal AscentState -----
        let mut state = AscentState::new();

        if let ::core::option::Option::Some(()) = opt_0 {
            let ev = /* &KeyEvent from event */;
            // body may state.invalidate(N); still returns InnerPath
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
        // ----- descent: schedule only -----
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

        // opt_2: #[bind(KeyA => outer_handler)] schedule token
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
            // into_parent creates snap BEFORE this runs; step_up AFTER it returns
            move |parent, snap| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = run_post(path, snap, |node, snap| {
                        after_child(id, node, snap)
                    });
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

Same indexed opts. Expands to `(noop_pre, post)`. User body is `(node, &AscentStateSnapshot)`. Never `claim()`.

## Prefactors

Behavior-identical until a step says otherwise. No unused parameters "for later."

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

- `type Effect` is the item (`MercuryEffect`)
- `dispatch(path, event, effs: &mut Vec<Effect>) -> ControlFlow<(), Path>`
- exclusive pushes onto `effs` and `Break(())`
- top-level `Some(effs)` on win, `None` on miss

### P1 — `on_into_parent` + sink together

`F` on `PathMut`, `into_parent` runs it and extends sink. All sites pass `empty_on_into_parent`. Behavior-identical.

### P2 — `from_fn` framework-only

### P3 — `AscentState` + `into_parent` body

Drop `Break`. Always return path. Leaf `AscentState::new()`; return `(path, state)`. `into_parent`: `snapshot` + posts + `step_up` (posts may be empty). Exclusive via `claim`. No user `invalidate` yet.

### P4 — `#[bind]` through `run_exclusive` only

Handlers return `(Vec<Effect>, P)` at the same level. Behavior-identical to P3.

### Feature steps

1. `#[post]` with `&AscentStateSnapshot`
2. `invalidate(N)` on kill (reshape carrier may still be open)
3. `#[pre_post]`
4. mercury rearm
5. reshape carrier: schedule field replace at owner without exclusive stealing PathMut

### Not prefactors

- Completion token
- Unused sink alone
- Root-owned reshape scheduler
- AndReturnHome restructure (needs a post)
- A second path-climbing API inside exclusive that returns a different path type

## Rules

1. Descent schedules; set is final (`opt_N` only).
2. Ascent runs every scheduled post; kill does not cancel posts.
3. `AscentState` is internal (`pub(crate)`). User posts receive `&AscentStateSnapshot` only.
4. Framework `into_parent` = `snapshot` → posts(`&snap`) → `step_up`.
5. Kill = `invalidate(N)` on live state (exclusive/framework); exclusive still returns same-level path; framework stack still walks.
6. Posts and exclusives are different signatures. Posts never see `AscentState`.
7. `claim` only in `run_exclusive`. Logging never claims.
8. Generate stays thin. Full expanded `Dispatch` impls are part of this doc.

## Tests

- post after deep bind sees `MaybeDropped` when `invalidate(N)` set depth > 0
- post sees `Intact` when no invalidate
- snapshot is pre-`step_up` for that hop
- each framework hop calls `step_up` once
- deepest exclusive wins claim; parent exclusive skips
- exclusive returns same path type it received (no climbed return type)
- pre return consumed once; pre miss → no post
- `only_if_intact` skips on `MaybeDropped`

## Open

- Reshape carrier: how exclusive schedules field replace at owner without consuming PathMut / changing return path type.
- Exact `N` for a given kill (who computes hops to owner — helper vs hand).
- Whether `pre` may push now-effects on the way down.
- Sugar so posts can write `-> Vec<Effect>` while derive threads path.
- Product nodes / multiple live children.
- Fallbacks if exclusive already claimed — deferred.
