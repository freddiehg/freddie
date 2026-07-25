# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

- **`AscentState`**: live hop counter + claim. Constructed at the leaf. Threaded as `&mut AscentState` through framework code and into **exclusive** bodies. Exclusive must take `&mut AscentState`: `run_exclusive` try-takes via `claim`, and the body must call `invalidate(N)` on the live object. A snapshot cannot do that.
- **`AscentStateSnapshot`**: frozen at `state.snapshot()` on entry to framework `into_parent` (after the child has returned, so deeper `claim` / `invalidate` are already applied). **Posts only** get `&AscentStateSnapshot`:
  - `snap.mutation() -> Intact | MaybeDropped`
  - `snap.claimed() -> bool` (read-only: deeper exclusive already took this event)

Two roles, two types. Posts do not get `AscentState`. Exclusive does not get only a snapshot.

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy)]
pub enum Mutation {
    /// Live `invalidation_depth` was 0 when this snapshot was taken.
    Intact,
    /// Live `invalidation_depth` was > 0 when this snapshot was taken.
    /// A deeper exclusive called `invalidate(N)` covering this hop.
    /// Child fields in that zone may already be gone; prefer pre carriage.
    MaybeDropped,
}

/// Token from successful `claim`. Not public API.
struct Claimed;

/// Live ascent machine state.
///
/// Public so exclusive handlers in app crates can take `&mut AscentState` and call
/// `invalidate`. Fields are private. Posts never receive this type — they get
/// `AscentStateSnapshot` only.
pub struct AscentState {
    invalidation_depth: u32,
    claim: Option<Claimed>,
}

/// Frozen view passed to user post functions. Both fields are set at `snapshot()`;
/// posts do not call `claim` / `invalidate` / `step_up`.
pub struct AscentStateSnapshot {
    mutation: Mutation,
    /// Whether a deeper exclusive already try-took this event (`claim` is Some).
    claimed: bool,
}

impl AscentState {
    /// Leaf turnaround only (framework).
    pub fn new() -> Self {
        Self {
            invalidation_depth: 0,
            claim: None,
        }
    }

    /// Framework `into_parent`, before posts. Freezes mutation + claimed.
    pub fn snapshot(&self) -> AscentStateSnapshot {
        AscentStateSnapshot {
            mutation: if self.invalidation_depth == 0 {
                Mutation::Intact
            } else {
                Mutation::MaybeDropped
            },
            claimed: self.claim.is_some(),
        }
    }

    /// Exclusive kill: `invalidation_depth = invalidation_depth.max(d)`.
    /// `d` = number of framework `into_parent` hops from this exclusive's level
    /// up through the reshape owner (inclusive).
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Framework `into_parent` only, after posts at this level.
    pub fn step_up(&mut self) {
        self.invalidation_depth = self.invalidation_depth.saturating_sub(1);
    }

    /// `run_exclusive` only. Try-take. Not a getter.
    pub fn claim(&mut self) -> Option<Claimed> {
        match self.claim {
            Some(_) => None,
            None => {
                self.claim = Some(Claimed);
                Some(Claimed)
            }
        }
    }

    /// Top-level `dispatch` only. Observe without try-taking.
    pub fn claimed(&self) -> bool {
        self.claim.is_some()
    }
}

impl AscentStateSnapshot {
    pub fn mutation(&self) -> Mutation {
        self.mutation
    }

    /// Deeper exclusive already took this event. Read-only; does not try-take.
    pub fn claimed(&self) -> bool {
        self.claimed
    }
}
```

## User function signatures

```rust
// pre — descent, shared path
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post — ascent; snapshot only (no &mut AscentState)
fn post(
    pre_return: T,
    node: Node<P, D>,
    snap: &AscentStateSnapshot,
) -> (Vec<M::Effect>, P);

// post alone (noop_pre): no pre_return arg
fn post(
    node: Node<P, D>,
    snap: &AscentStateSnapshot,
) -> (Vec<M::Effect>, P);

// exclusive — run_exclusive; live state for claim + invalidate
fn exclusive(
    ev: &SourceEvent,
    node: Node<P, D>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, P);
```

Return path is always the **same level** `P` the handler received.

## Attr → schedule

```rust
#[pre_post(trig => (pre, post))]  // opt = Some(pre(...))
#[post(trig => post)]             // opt = Some(noop_pre(...)) i.e. Some(())
#[bind(trig => handler)]          // opt = Some(noop_pre(...)); ascent: run_exclusive(handler)
```

```rust
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}
```

## Framework helpers (`crates/bind`, not generated per node)

```rust
pub fn run_post<P, Effect>(
    path: P,
    snap: &AscentStateSnapshot,
    body: impl FnOnce(Node<P, ()>, &AscentStateSnapshot) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    body(Node { parent: path, data: () }, snap)
}

pub fn run_exclusive<P, Effect>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match state.claim() {
        None => (path, Vec::new()),
        Some(Claimed) => body(Node { parent: path, data: () }, state),
    }
}

pub fn only_if_intact<P, N, Effect>(
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

## `PathMut` (laserbeam) + ascent hop (bind)

laserbeam must not name `AscentState` / `AscentStateSnapshot` (bind depends on laserbeam, not reverse). PathMut is generic over the post context `C` and effect item `E`.

### Before (master today)

```rust
// crates/laserbeam
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
    ) -> Self { /* … */ }

    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

### After P1 — posts + sink, no ascent state yet

```rust
// crates/laserbeam
pub struct PathMut<Node, Parent, F> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
    shared: ProjRef<Node, Parent>,
    on_into_parent: F,
}

impl<Node, Parent, F> PathMut<Node, Parent, F> {
    pub fn from_fn<E>(
        parent: Parent,
        projection: fn(&mut Parent) -> &mut Node,
        shared: fn(&Parent) -> &Node,
        on_into_parent: F,
    ) -> Self
    where
        F: FnOnce(Parent) -> (Parent, Vec<E>),
    {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
            shared: ProjRef::Bare(shared),
            on_into_parent,
        }
    }

    pub fn into_parent<E>(self, sink: &mut Vec<E>) -> Parent
    where
        F: FnOnce(Parent) -> (Parent, Vec<E>),
    {
        let (parent, post_effs) = (self.on_into_parent)(self.parent);
        sink.extend(post_effs);
        parent
    }
}

// every site until posts exist:
fn empty_on_into_parent<P, E>(parent: P) -> (P, Vec<E>) {
    (parent, Vec::new())
}
```

### After P3 — post context `C` (bind passes `AscentStateSnapshot`)

```rust
// crates/laserbeam — still no bind types
impl<Node, Parent, F> PathMut<Node, Parent, F> {
    pub fn from_fn<C, E>(
        parent: Parent,
        projection: fn(&mut Parent) -> &mut Node,
        shared: fn(&Parent) -> &Node,
        on_into_parent: F,
    ) -> Self
    where
        F: FnOnce(Parent, &C) -> (Parent, Vec<E>),
    {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
            shared: ProjRef::Bare(shared),
            on_into_parent,
        }
    }

    /// Runs posts with `ctx`, extends `sink`, returns parent. Does **not** know AscentState.
    pub fn into_parent<C, E>(self, ctx: &C, sink: &mut Vec<E>) -> Parent
    where
        F: FnOnce(Parent, &C) -> (Parent, Vec<E>),
    {
        let (parent, post_effs) = (self.on_into_parent)(self.parent, ctx);
        sink.extend(post_effs);
        parent
    }
}

// crates/bind — snapshot + step_up live here
pub fn into_parent_ascent<Node, Parent, F, E>(
    path: PathMut<Node, Parent, F>,
    sink: &mut Vec<E>,
    state: &mut AscentState,
) -> Parent
where
    F: FnOnce(Parent, &AscentStateSnapshot) -> (Parent, Vec<E>),
{
    let snap = state.snapshot();
    let parent = path.into_parent(&snap, sink);
    state.step_up();
    parent
}

fn empty_on_into_parent<P, C, E>(parent: P, _ctx: &C) -> (P, Vec<E>) {
    (parent, Vec::new())
}
```

Generated Outer calls `::bind::into_parent_ascent(inner_path, effs, &mut state)`, not a laserbeam method that names `AscentState`.

## `Dispatch` / top-level

Before (batch threaded, `Break` still — after P0):

```rust
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

After:

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
  for each attr i: if trigger matches, opt_i = Some(pre_or_noop_pre(...)) else None

  if has resolve_into child:
    child_path = PathMut::from_fn(path, proj_mut, proj_ref, on_into_parent)
      // on_into_parent: FnOnce(Parent, &AscentStateSnapshot) -> (Parent, Vec<Effect>)
    (child_path, state) = Child::dispatch(child_path, event, effs)
    path = bind::into_parent_ascent(child_path, effs, &mut state)
      // snap = state.snapshot()
      // parent = laserbeam into_parent(&snap, sink)
      // state.step_up()

  if leaf (no child):
    state = AscentState::new()

  if exclusive scheduled (opt holds &SourceEvent from descent TryFrom):
    run_exclusive(path, &mut state, |node, state| handler(ev, node, state))

  return (path, state)
```

## Kill

Same-level path return. Framework `PathMut` stack still walks and still runs ancestor posts. `invalidate(N)` is on the live `AscentState` the exclusive receives. Reshape scheduling is F5; until then `invalidate` alone is enough for snapshot/post behavior.

## DX example (the tree the expand is for)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
struct ChildId(u64);

struct AndReturnHome {
    guard: TimerGuard,
}

// demo effect item; M::Effect in real bind
enum DemoEffect {
    LogDestroyed(ChildId),
    ScheduleTimer(TimerId),
    SetLayerHome,
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
    snap: &AscentStateSnapshot,
) -> (Vec<DemoEffect>, OuterPath) {
    match snap.mutation() {
        Mutation::Intact => {
            let _live = node.parent.get().inner.id;
            debug_assert_eq!(_live, id);
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

/// Parent exclusive: only runs if Inner did not claim (run_exclusive gate).
/// Uses live `state` only if it needs invalidate; this handler does not kill.
fn outer_handler(
    _ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    _state: &mut AscentState,
) -> (Vec<DemoEffect>, OuterPath) {
    (vec![DemoEffect::SetLayerHome], node.parent)
}

/// Child exclusive: claims via run_exclusive, then invalidates a 2-hop spine.
/// Path return is still InnerPath — stack walks Outer (and above) afterward.
fn inner_handler(
    _ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<DemoEffect>, InnerPath) {
    state.invalidate(2);
    (vec![], node.parent)
}
```

### Generated code for the DX tree

`M` is a `Bindings` marker with `type Effect = DemoEffect` (or any effect item the handlers return; must be the same type in `effs`).

Exact expand. Review this block.

#### Inner

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
        // attr 0: #[bind(KeyA => inner_handler)]
        // Capture &KeyEvent once — do not TryFrom again on the ascent.
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

#### Outer

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
        // attr 0: #[pre_post(AnyKey => (snap_child_id, after_child))]
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

        // attr 1: #[post(AnyKey => only_if_intact(...))]
        let opt_1: bool =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = AnyKey;
                ::bind::EventTrigger::is_matching(&trigger, ev)
            } else {
                false
            };

        // attr 2: #[bind(KeyA => outer_handler)] — capture &KeyEvent once
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

        // Child path. F: FnOnce(OuterPath, &AscentStateSnapshot) -> (OuterPath, Vec<M::Effect>)
        // C = AscentStateSnapshot is only in F's type, not inside laserbeam's definition.
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p: &mut <Outer as ::bind::Place>::Path<'a>| &mut p.get_mut().inner,
            |p: &<Outer as ::bind::Place>::Path<'a>| &p.get().inner,
            move |
                parent: <Outer as ::bind::Place>::Path<'a>,
                snap: &::bind::AscentStateSnapshot,
            | {
                let mut local = ::std::vec::Vec::<<M as ::bind::Bindings>::Effect>::new();
                let mut path = parent;

                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = ::bind::run_post(path, snap, |node, snap| {
                        after_child(id, node, snap)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }

                if opt_1 {
                    let (p, e) = ::bind::run_post(path, snap, |node, snap| {
                        ::bind::only_if_intact(
                            |p: &mut <Outer as ::bind::Place>::Path<'a>| {
                                &mut p.get_mut().return_home
                            },
                            rearm,
                        )(node, snap)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }

                (path, local)
            },
        );

        let (inner_path, mut state) =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs);

        // bind::into_parent_ascent =
        //   let snap = state.snapshot();
        //   let parent = path.into_parent(&snap, sink);  // laserbeam
        //   state.step_up();
        //   parent
        let mut path =
            ::bind::into_parent_ascent(inner_path, effs, &mut state);

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

#### Helpers (bind library)

```rust
pub fn noop_pre<E, P, D>(_ev: &E, _node: ::bind::Node<&P, D>) {}

pub fn run_post<P, Effect>(
    path: P,
    snap: &AscentStateSnapshot,
    body: impl FnOnce(
        ::bind::Node<P, ()>,
        &AscentStateSnapshot,
    ) -> (::std::vec::Vec<Effect>, P),
) -> (P, ::std::vec::Vec<Effect>) {
    body(
        ::bind::Node {
            parent: path,
            data: (),
        },
        snap,
    )
}

pub fn run_exclusive<P, Effect>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(
        ::bind::Node<P, ()>,
        &mut AscentState,
    ) -> (::std::vec::Vec<Effect>, P),
) -> (P, ::std::vec::Vec<Effect>) {
    match state.claim() {
        ::core::option::Option::None => (path, ::std::vec::Vec::new()),
        ::core::option::Option::Some(Claimed) => body(
            ::bind::Node {
                parent: path,
                data: (),
            },
            state,
        ),
    }
}

pub fn only_if_intact<P, N, Effect>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> ::std::vec::Vec<Effect>,
) -> impl FnOnce(
    ::bind::Node<P, ()>,
    &AscentStateSnapshot,
) -> (::std::vec::Vec<Effect>, P) {
    move |mut node, snap| {
        let effects = match snap.mutation() {
            Mutation::Intact => f(project(&mut node.parent)),
            Mutation::MaybeDropped => ::std::vec::Vec::new(),
        };
        (effects, node.parent)
    }
}

pub fn into_parent_ascent<Node, Parent, F, Effect>(
    path: ::laserbeam::PathMut<Node, Parent, F>,
    sink: &mut ::std::vec::Vec<Effect>,
    state: &mut AscentState,
) -> Parent
where
    F: FnOnce(Parent, &AscentStateSnapshot) -> (Parent, ::std::vec::Vec<Effect>),
{
    let snap = state.snapshot();
    let parent = path.into_parent(&snap, sink);
    state.step_up();
    parent
}
```

### Walk: KeyA, Inner `invalidate(2)`

Tree: Root contains Outer contains Inner.

```text
DESCENT Outer
  opt_0 = Some(snap_child_id(...))     // AnyKey pre_post
  opt_1 = Some(())                     // AnyKey post (noop_pre)
  opt_2 = Some(())                     // KeyA bind (noop_pre)
  from_fn → InnerPath

DESCENT Inner
  opt_0 = Some(())                     // KeyA bind

ASCENT Inner (leaf)
  state = AscentState::new()             // depth 0, claim None
  run_exclusive:
    claim() → Some(Claimed)
    inner_handler: invalidate(2)         // depth = 2
  return (InnerPath, state)

ASCENT Outer into_parent (inlined)
  snap = state.snapshot()
    // mutation: MaybeDropped (depth 2)
    // claimed: true
  after_child(id, node, snap)
    // match MaybeDropped → vec![LogDestroyed(id)], return OuterPath
  only_if_intact(rearm)
    // MaybeDropped → vec![], guard Drop cancels old timer
  state.step_up()                        // depth 2 → 1

ASCENT Outer exclusive
  run_exclusive:
    claim() → None                       // Inner already took it
    outer_handler not called
    effs unchanged

return (OuterPath, state) with depth 1, claim Some
// further ancestors (Root) each into_parent: snapshot, their posts, step_up
// until depth reaches 0
```

### Walk: KeyB (AnyKey only; no KeyA bind)

```text
DESCENT Outer: opt_0 Some(id), opt_1 Some(()), opt_2 None
DESCENT Inner: opt_0 None

ASCENT Inner:
  state = AscentState::new()             // depth 0, claim None
  no exclusive
  return (InnerPath, state)

ASCENT Outer into_parent:
  snap = Intact; claimed false
  after_child Intact → assert id matches live .inner.id; no effects
  only_if_intact → rearm:
    arm_return_home(); child.guard = guard; vec![ScheduleTimer(id)]
  step_up (depth stays 0)
  no outer exclusive (opt_2 None)

return (OuterPath, state) claim None, depth 0
```

## Ordered changes

Each ships alone. Behavior-identical until a step says otherwise.

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

Before:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}
// dispatch(path, event) -> ControlFlow<M::Output, Path>
```

After:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}
// dispatch(path, event, effs: &mut Vec<M::Effect>) -> ControlFlow<(), Path>
// exclusive pushes onto effs, Break(())
// top-level Some(effs) / None
```

### P1 — `on_into_parent` + sink together

Before (master today):

```rust
pub fn into_parent(self) -> Parent { /* project up */ }
```

After:

```rust
pub fn into_parent(self, sink: &mut Vec<Effect>) -> Parent {
    let (parent, post_effs) = (self.on_into_parent)(self.parent);
    Extend::extend(sink, post_effs);
    parent
}
// every site: on_into_parent = empty_on_into_parent
fn empty_on_into_parent<P>(parent: P) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}
```

### P2 — `from_fn` framework-only

Crate-private / sealed. No user call sites outside laserbeam/bind.

### P3 — `AscentState` + `into_parent_ascent` snapshot / step_up

Before: P1 laserbeam `into_parent(sink)` with `F: FnOnce(Parent) -> (Parent, Vec<E>)`.

After:
- laserbeam `F: FnOnce(Parent, &C) -> (Parent, Vec<E>)`, `into_parent(&C, sink)`
- bind `into_parent_ascent` = `snapshot` + laserbeam `into_parent` + `step_up`
- Dispatch returns `(Path, AscentState)`; leaf `AscentState::new()`
- Exclusive via `run_exclusive`
- Top-level `claimed() || !effs.is_empty()`
- No user posts yet (`empty_on_into_parent`)

### P4 — `#[bind]` only through `run_exclusive`

Before: today's immediate exclusive `Break` path.

After: schedule on descent; `run_exclusive` on ascent; handler `fn(ev, node, &mut AscentState) -> (Vec<Effect>, P)` with same-level `P`. Behavior-identical to P3 for non-kill handlers.

### F1 — `#[post]`

Before: empty `on_into_parent`.

After: user post `fn(node, &AscentStateSnapshot) -> (Vec, P)` (and expression form). Generate fills `on_into_parent` as in Outer expand attrs 0/1.

### F2 — `invalidate(N)`

Before: exclusive cannot mark spine depth.

After: exclusive body calls `state.invalidate(N)`; ancestor posts see `MaybeDropped` via snapshot. Expand: `inner_handler` as in DX.

### F3 — `#[pre_post]` + `noop_pre`

Before: post alone has no pre half helper.

After: `noop_pre`; `#[post]` = `(noop_pre, post)`; `#[pre_post]` threads pre return into post first arg. Expand: Outer `opt_0` / `after_child`.

### F4 — `only_if_intact` + mercury rearm

Before: rearm via other means (handle discriminant, etc.).

After:

```rust
#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]
fn rearm(child: &mut AndReturnHome) -> Vec<DemoEffect> { /* as DX */ }
```

### F5 — reshape carrier

Before: exclusive `invalidate` only; child field value still whatever was there.

After: exclusive still returns same-level `P` and still only `invalidate(N)` for hop depth. Field replace at owner is a separate effect / model write already expressible today (`set_layer` via `ascend` after posts), or a later carrier — not required for snapshot/post correctness. When a dedicated carrier is designed, add full types and expand here as F5b; do not invent it mid-implement.

## Rules

1. Descent schedules `opt_0`… only; set is final.
2. Ascent runs every scheduled post.
3. Posts: `&AscentStateSnapshot` only. Exclusive: `&mut AscentState`.
4. laserbeam never names bind ascent types. bind `into_parent_ascent` = `snapshot` → laserbeam `into_parent(&snap, sink)` → `step_up`.
5. Kill = `invalidate(N)` on live state; same-level path return; stack still walks.
6. Exclusive schedule stores `&SourceEvent` from descent `TryFrom` (no second TryFrom).
7. `claim` only inside `run_exclusive`.
8. Generate stays thin: schedule + call helpers. The expand for the DX tree above is the template.

## Tests (implement after the matching feature step)

- post after deep bind sees `MaybeDropped` when `invalidate(N)` set depth > 0
- post sees `Intact` when no invalidate
- post after deep exclusive sees `snap.claimed() == true`
- post with no exclusive below sees `snap.claimed() == false`
- snapshot is pre-`step_up` for that hop (claimed already final from child)
- each framework hop calls `step_up` once
- deepest exclusive wins claim; parent exclusive skips
- exclusive returns same path type it received
- pre return consumed once; pre miss → no post
- `only_if_intact` skips on `MaybeDropped`
- KeyA walk and KeyB walk match the traces above
