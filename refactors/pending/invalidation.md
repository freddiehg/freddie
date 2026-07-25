# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

One type: **`AscentState`**, passed as **`&mut AscentState`** to posts and exclusives.

- **`mutation()`** — frozen at a point in time (`freeze_mutation` at the start of each `into_parent_ascent`). Intact / MaybeDropped from `invalidation_depth` at freeze. Posts read this; it does not change mid-post-batch when `step_up` runs after.
- **`claim()`** — one-way trap door. Try-take. `claimed()` is the read. Once taken, stays taken.
- **`invalidate(d)`** / **`step_up()`** — mutate the live hop counter only through these methods.

No second “snapshot” type. No separate frozen bag for posts.

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    /// `invalidation_depth == 0` at the last `freeze_mutation`.
    Intact,
    /// `invalidation_depth > 0` at the last `freeze_mutation`.
    /// A deeper exclusive called `invalidate(N)` covering this hop.
    MaybeDropped,
}

/// Proof `claim()` succeeded. Private constructor except via `claim`.
#[derive(Clone, Copy, Debug)]
pub struct Claimed {
    _seal: (),
}

pub struct AscentState {
    invalidation_depth: u32,
    claim: Option<Claimed>,
    /// Set only by `freeze_mutation`. What `mutation()` returns.
    frozen_mutation: Mutation,
}

impl AscentState {
    pub fn new() -> Self {
        Self {
            invalidation_depth: 0,
            claim: None,
            frozen_mutation: Mutation::Intact,
        }
    }

    /// Call at the start of each framework hop (before posts).
    /// Freezes Intact/MaybeDropped from the current hop counter.
    pub fn freeze_mutation(&mut self) {
        self.frozen_mutation = if self.invalidation_depth == 0 {
            Mutation::Intact
        } else {
            Mutation::MaybeDropped
        };
    }

    pub fn mutation(&self) -> Mutation {
        self.frozen_mutation
    }

    /// One-way trap door. `Some(Claimed)` if open (now taken). `None` if already taken.
    pub fn claim(&mut self) -> Option<Claimed> {
        if self.claim.is_some() {
            None
        } else {
            self.claim = Some(Claimed { _seal: () });
            self.claim
        }
    }

    pub fn claimed(&self) -> bool {
        self.claim.is_some()
    }

    /// Exclusive kill: `invalidation_depth = invalidation_depth.max(d)`.
    /// Does not change `frozen_mutation` (current level’s freeze stays).
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// After posts at this level. Does not change `frozen_mutation`.
    pub fn step_up(&mut self) {
        self.invalidation_depth = self.invalidation_depth.saturating_sub(1);
    }
}
```

## User signatures

```rust
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

fn post(
    pre_return: T,
    node: Node<P, D>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, P);

fn post(
    node: Node<P, D>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, P);

fn exclusive(
    ev: &SourceEvent,
    node: Node<P, D>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, P);
```

Same `P` in and out. Posts and exclusive both take `&mut AscentState`.

## Attr → schedule

```rust
#[pre_post(trig => (pre, post))]  // opt = Some(pre(...))
#[post(trig => post)]             // opt = Some(()) via match only
#[bind(trig => handler)]          // opt = Some(&SourceEvent) captured at match
```

## PathMut (laserbeam)

Master shape. Parent + projections. `into_parent` returns the parent.

```rust
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
    shared: ProjRef<Node, Parent>,
}

impl<Node, Parent> PathMut<Node, Parent> {
    pub const fn from_fn(
        parent: Parent,
        projection: fn(&mut Parent) -> &mut Node,
        shared: fn(&Parent) -> &Node,
    ) -> Self {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
            shared: ProjRef::Bare(shared),
        }
    }

    pub fn get(&self) -> &Node {
        self.shared.apply(&self.parent)
    }

    pub fn get_mut(&mut self) -> &mut Node {
        self.projection.apply(&mut self.parent)
    }

    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

## `into_parent_ascent` (bind)

```rust
pub fn into_parent_ascent<Node, Parent, E>(
    path: laserbeam::PathMut<Node, Parent>,
    sink: &mut Vec<E>,
    state: &mut AscentState,
    run_posts: impl FnOnce(Parent, &mut AscentState) -> (Parent, Vec<E>),
) -> Parent {
    state.freeze_mutation();
    let parent = path.into_parent();
    let (parent, post_effs) = run_posts(parent, state);
    sink.extend(post_effs);
    state.step_up();
    parent
}
```

## Helpers (bind)

```rust
pub fn run_post<P, E>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<E>, P),
) -> (P, Vec<E>) {
    body(Node { parent: path, data: () }, state)
}

pub fn run_exclusive<P, E>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<E>, P),
) -> (P, Vec<E>) {
    match state.claim() {
        None => (path, Vec::new()),
        Some(_claimed) => body(Node { parent: path, data: () }, state),
    }
}

pub fn only_if_intact<P, N, E>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<E>,
) -> impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<E>, P) {
    move |mut node, state| {
        let effects = match state.mutation() {
            Mutation::Intact => f(project(&mut node.parent)),
            Mutation::MaybeDropped => Vec::new(),
        };
        (effects, node.parent)
    }
}
```

## Dispatch

### Before (master)

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
    ) -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
    }
}
```

### After P0 (batch, still Break)

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> ControlFlow<(), Self::Path<'a>>
    where
        Self: 'a;
}
```

### After P3 (ascent state)

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> (Self::Path<'a>, AscentState)
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(
    path: N::Path<'a>,
    event: &M::Event,
) -> Option<Vec<M::Effect>>
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

## Level order

```text
DESCENT
  schedule opts (pre values / exclusive &Event / post bool)

  if child:
    path = PathMut::from_fn(path, proj_mut, proj_ref)   // 3-arg, no F
    (path, state) = Child::dispatch(path, event, effs)
    path = into_parent_ascent(path, effs, &mut state, |parent, state| {
      // posts; opts captured from descent
      (parent, local_effs)
    })
      // freeze_mutation(); parent = path.into_parent(); run_posts; step_up()

  if leaf:
    state = AscentState::new()

  if exclusive scheduled:
    run_exclusive(path, &mut state, handler)

  return (path, state)
```

## DX example

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChildId(u64);

struct TimerGuard {
    id: TimerId,
}

struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        // demo only
        Self(1)
    }
}

impl TimerGuard {
    fn armed(id: TimerId) -> Self {
        Self { id }
    }
}

struct AndReturnHome {
    guard: TimerGuard,
}

enum DemoEffect {
    LogDestroyed(ChildId),
    ScheduleTimer(TimerId),
    SetLayerHome,
}

struct M;

impl Bindings for M {
    type Trigger = /* key trigger */;
    type Event = DemoEvent;
    type Effect = DemoEffect;
}

fn log_destroyed(id: ChildId) -> DemoEffect {
    DemoEffect::LogDestroyed(id)
}

fn arm_return_home() -> (TimerGuard, DemoEffect) {
    let id = TimerId::fresh();
    (TimerGuard::armed(id), DemoEffect::ScheduleTimer(id))
}

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

fn snap_child_id(_ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

fn after_child(
    id: ChildId,
    node: Node<OuterPath, ()>,
    state: &mut AscentState,
) -> (Vec<DemoEffect>, OuterPath) {
    match state.mutation() {
        Mutation::Intact => {
            let live = node.parent.get().inner.id;
            debug_assert_eq!(live, id);
            (vec![], node.parent)
        }
        Mutation::MaybeDropped => (vec![log_destroyed(id)], node.parent),
    }
}

fn rearm(child: &mut AndReturnHome) -> Vec<DemoEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

fn outer_handler(
    _ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    _state: &mut AscentState,
) -> (Vec<DemoEffect>, OuterPath) {
    (vec![DemoEffect::SetLayerHome], node.parent)
}

fn inner_handler(
    _ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<DemoEffect>, InnerPath) {
    state.invalidate(2);
    (vec![], node.parent)
}
```

## Generated: Inner

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
    ) -> (
        <Inner as ::bind::Place>::Path<'a>,
        ::bind::AscentState,
    )
    where
        Self: 'a,
    {
        let opt_0: ::core::option::Option<&KeyEvent> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = KeyA;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some(ev)
                } else {
                    ::core::option::Option::None
                }
            } else {
                ::core::option::Option::None
            };

        let mut state = ::bind::AscentState::new();

        if let ::core::option::Option::Some(ev) = opt_0 {
            let (path, out_effs) = ::bind::run_exclusive(path, &mut state, |node, state| {
                inner_handler(ev, node, state)
            });
            ::core::iter::Extend::extend(effs, out_effs);
            return (path, state);
        }

        (path, state)
    }
}
```

## Generated: Outer

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Outer
where
    Inner: ::bind::Dispatch<M>,
{
    fn dispatch<'a>(
        mut path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
    ) -> (
        <Outer as ::bind::Place>::Path<'a>,
        ::bind::AscentState,
    )
    where
        Self: 'a,
    {
        let opt_0: ::core::option::Option<ChildId> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
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

        let opt_1: bool =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = AnyKey;
                ::bind::EventTrigger::is_matching(&trigger, ev)
            } else {
                false
            };

        let opt_2: ::core::option::Option<&KeyEvent> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = KeyA;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some(ev)
                } else {
                    ::core::option::Option::None
                }
            } else {
                ::core::option::Option::None
            };

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p: &mut <Outer as ::bind::Place>::Path<'a>| &mut p.get_mut().inner,
            |p: &<Outer as ::bind::Place>::Path<'a>| &p.get().inner,
        );

        let (inner_path, mut state) =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs);

        let mut path = ::bind::into_parent_ascent(
            inner_path,
            effs,
            &mut state,
            move |parent, state| {
                let mut local =
                    ::std::vec::Vec::<<M as ::bind::Bindings>::Effect>::new();
                let mut path = parent;

                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = ::bind::run_post(path, state, |node, state| {
                        after_child(id, node, state)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }

                if opt_1 {
                    let (p, e) = ::bind::run_post(path, state, |node, state| {
                        ::bind::only_if_intact(
                            |p: &mut <Outer as ::bind::Place>::Path<'a>| {
                                &mut p.get_mut().return_home
                            },
                            rearm,
                        )(node, state)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }

                (path, local)
            },
        );

        if let ::core::option::Option::Some(ev) = opt_2 {
            let (p, e) = ::bind::run_exclusive(path, &mut state, |node, state| {
                outer_handler(ev, node, state)
            });
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }

        (path, state)
    }
}
```

## Walk: KeyA, Inner `invalidate(2)`

```text
DESCENT Outer: opt_0 Some(id), opt_1 true, opt_2 Some(&KeyA event)
DESCENT Inner: opt_0 Some(&KeyA event)

ASCENT Inner
  state = AscentState::new()
    depth 0, claim None, frozen Intact
  run_exclusive:
    claim() → Some(Claimed)
    inner_handler: invalidate(2)         // depth = 2; frozen still Intact
  return (InnerPath, state)

ASCENT Outer into_parent_ascent
  freeze_mutation()                      // frozen = MaybeDropped (depth 2)
  on_into_parent(&mut state):
    after_child: mutation() MaybeDropped → LogDestroyed(id)
    only_if_intact: MaybeDropped → no rearm
  step_up()                              // depth 2 → 1; frozen still MaybeDropped

ASCENT Outer exclusive
  claim() → None                         // trap door already shut
  outer_handler not run

return (OuterPath, state) depth 1, claimed true
// ancestor hops: freeze_mutation from depth, posts, step_up until depth 0
```

## Walk: KeyB (AnyKey only)

```text
DESCENT Outer: opt_0 Some(id), opt_1 true, opt_2 None
DESCENT Inner: opt_0 None

ASCENT Inner
  state = new()
  no exclusive

ASCENT Outer into_parent_ascent
  freeze_mutation()                      // Intact (depth 0)
  after_child: Intact, assert live id
  only_if_intact: rearm → ScheduleTimer
  step_up()                              // depth 0

  no exclusive

return claim None, depth 0
```

## Ordered changes

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

Before: `type Output`; `Break(output)`.

After: `type Effect`; `effs: &mut Vec<Effect>`; `Break(())`; top-level `Some(effs)` / `None`.

### P1 — sink only (optional prefactor)

If posts need a sink before ascent state: thread `effs` through dispatch. PathMut shape unchanged.

### P2 — `from_fn` framework-only

### P3 — `AscentState` + `into_parent_ascent`

- Dispatch returns `(Path, AscentState)`.
- Leaf `AscentState::new()`.
- `into_parent_ascent(path, sink, state, run_posts)` as written above.
- Exclusive via `run_exclusive` / trap-door `claim`.
- Top-level `claimed() || !effs.is_empty()`.

### P4 — `#[bind]` through `run_exclusive`

Handlers `(ev, node, &mut AscentState) -> (Vec, P)`. Same-level path.

### F1 — `#[post]` with `&mut AscentState`

Posts read `state.mutation()` (frozen for this hop). `run_posts` closure is the generate site (Outer expand).

### F2 — `invalidate(N)`

Exclusive kill: `state.invalidate(N)`. Next hop’s `freeze_mutation` sees MaybeDropped.

### F3 — `#[pre_post]`

Pre return threaded into post first arg.

### F4 — `only_if_intact` + rearm

```rust
#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]
```

### F5 — reshape carrier

Exclusive still returns same-level `P`. Field replace at owner is a later carrier; specify full types before implementing.

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. One `AscentState`. Posts and exclusive take `&mut AscentState`.
4. `mutation()` is frozen at `freeze_mutation` (into_parent_ascent entry).
5. `claim()` is a one-way trap door; `claimed()` reads it.
6. `into_parent_ascent` = freeze → laserbeam `into_parent` → run_posts → step_up.
7. Kill = `invalidate(N)` on live depth; path type unchanged.
8. Generate: schedule + helpers. Expand above is the template.

## Tests

- after `invalidate(N)`, next hop `freeze_mutation` → `MaybeDropped`
- no invalidate → `Intact`
- `claim` then parent `claim` is None
- `claimed()` true after successful claim
- freeze then `step_up` leaves `mutation()` unchanged until next freeze
- exclusive returns same path type
- pre return once; pre miss → no post
- `only_if_intact` skips on MaybeDropped
- KeyA / KeyB walks match above
