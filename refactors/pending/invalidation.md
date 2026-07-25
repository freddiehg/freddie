# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

After a kill, scheduled posts still run. At that point the live tree may already be gone, so the framework cannot mint a fresh `PathMut` for the post. The path that descent built and ascent recovered is therefore a field on the same value that carries invalidation: posts read path and `mutation()` from one struct.

```rust
// Top-level dispatch owns the claim slot only:
//   let mut claim_slot: Option<()> = None;
//   let mut claim = Claim::new(&mut claim_slot);

/// Path + hop counters. One value. Path is not rebuilt after invalidate.
pub struct Ascent<P> {
    path: P,
    invalidation_depth: u32,
    ascent_hops: u32,
}

/// Exclusive trap door. Separate carrier; not a field of Ascent.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}
```

Two carriers, different jobs:

- `Ascent<P>` owns the path and the depth counters. `invalidate` writes `invalidation_depth`. `into_parent_ascent` moves the path through `into_parent`, bumps `ascent_hops`, rebuilds `Ascent<Parent>` with the same counters. Posts receive `&mut Ascent<P>` and already have the path that was recovered for this level — including when `mutation()` is `MaybeDropped`.
- `Claim<'c>` closes over the root claim slot. Exclusive is `claim.with_exclusive(&mut ascent, body)`. Claim never sits on `Ascent`, so path's interior `&mut Root` and the claim reborrow are different places.

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

Depth is live on `Ascent`. After `invalidate(d)` or a hop bumps `ascent_hops`, the next `mutation()` sees the new numbers.

laserbeam `PathMut::into_parent` recovers parent only (path value already held; no new path from the live tree).

Kill: `ascent.invalidate(d)` with `d` = kill path hop count. Posts still run; they use this `Ascent`'s path field.

Methods:

- on `Ascent`: `path` / `path_mut`, `mutation`, `invalidate`, `into_parent_ascent` (framework)
- on `Claim`: `try_take`, `is_taken`, `with_exclusive`

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

/// Path is part of this value. After invalidate, posts still hold it here.
pub struct Ascent<P> {
    path: P,
    invalidation_depth: u32,
    ascent_hops: u32,
}

impl<P> Ascent<P> {
    pub fn new(path: P) -> Self {
        Self {
            path,
            invalidation_depth: 0,
            ascent_hops: 0,
        }
    }

    pub fn path(&self) -> &P {
        &self.path
    }

    pub fn path_mut(&mut self) -> &mut P {
        &mut self.path
    }

    pub fn mutation(&self) -> Mutation {
        if self.ascent_hops < self.invalidation_depth {
            Mutation::MaybeDropped
        } else {
            Mutation::Intact
        }
    }

    /// Kill coverage. d = path hops of the kill climb.
    /// Does not drop the path. Posts still run against this Ascent.
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }
}

impl<Node, Parent> Ascent<laserbeam::PathMut<Node, Parent>> {
    /// Framework hop: consume path → parent, bump hops, run posts on Ascent<Parent>.
    /// Claim is not involved. Path is the recovered parent, not a new mint from the live tree.
    pub fn into_parent_ascent<E>(
        self,
        sink: &mut Vec<E>,
        run_posts: impl FnOnce(&mut Ascent<Parent>) -> Vec<E>,
    ) -> Ascent<Parent> {
        let Ascent {
            path,
            invalidation_depth,
            mut ascent_hops,
        } = self;
        let parent = path.into_parent();
        ascent_hops = ascent_hops.saturating_add(1);
        let mut ascent = Ascent {
            path: parent,
            invalidation_depth,
            ascent_hops,
        };
        let post_effs = run_posts(&mut ascent);
        sink.extend(post_effs);
        ascent
    }
}

/// Closes over the root claim slot. Not on Ascent.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

impl<'c> Claim<'c> {
    pub fn new(slot: &'c mut Option<()>) -> Self {
        Self { slot }
    }

    pub fn is_taken(&self) -> bool {
        self.slot.is_some()
    }

    /// One-way trap door. Some(()) if this call took it.
    pub fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }

    /// Exclusive gate on the claim side; body gets Ascent (path + depth).
    pub fn with_exclusive<P, E>(
        &mut self,
        ascent: &mut Ascent<P>,
        body: impl FnOnce(&mut Ascent<P>) -> Vec<E>,
    ) -> Vec<E> {
        match self.try_take() {
            None => Vec::new(),
            Some(()) => body(ascent),
        }
    }
}
```

Hop accounting (worked `invalidate(2)` from the leaf):

```text
leaf:     hops=0 inv=2  →  0 < 2  MaybeDropped
+1 hop:   hops=1 inv=2  →  1 < 2  MaybeDropped   // Outer posts here; path is OuterPath on Ascent
+2 hops:  hops=2 inv=2  →  2 < 2  Intact
```

Order invariant inside `into_parent_ascent`: bump before posts. Framework never writes `invalidation_depth`; only `invalidate` does.

## User signatures

```rust
// pre — descent; no Ascent yet
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post — always runs if scheduled, including after invalidate.
// Path is ascent.path(); it was recovered on the way up, not minted here.
fn post(pre_return: T, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
fn post(ascent: &mut Ascent<P>) -> Vec<M::Effect>;

// exclusive — body sees Ascent; claim gate is Claim::with_exclusive
fn exclusive(ev: &SourceEvent, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
```

## PathMut (laserbeam)

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

## Helpers (bind)

```rust
pub fn run_post<P, E>(
    ascent: &mut Ascent<P>,
    body: impl FnOnce(&mut Ascent<P>) -> Vec<E>,
) -> Vec<E> {
    body(ascent)
}

pub fn only_if_intact<P, N, E>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<E>,
) -> impl FnOnce(&mut Ascent<P>) -> Vec<E> {
    move |ascent| match ascent.mutation() {
        Mutation::Intact => f(project(ascent.path_mut())),
        Mutation::MaybeDropped => Vec::new(),
    }
}
```

## Dispatch

Root owns the claim slot. `Ascent` is built at the leaf with the path and zero counters, returned up the spine. `Claim` is threaded down for exclusive only.

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Ascent<Self::Path<'a>>
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
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _ascent = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    if claim.is_taken() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

## Level order

```text
DESCENT: schedule opts

if child:
  child_path = PathMut::from_fn(path, proj_mut, proj_ref)
  ascent = Child::dispatch(child_path, event, effs, claim)
  ascent = ascent.into_parent_ascent(effs, |ascent| { /* posts */ })

if leaf:
  ascent = Ascent::new(path)

if exclusive scheduled:
  effs2 = claim.with_exclusive(&mut ascent, |ascent| handler(ev, ascent))
  extend effs

return ascent
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

fn after_child(id: ChildId, ascent: &mut Ascent<OuterPath>) -> Vec<DemoEffect> {
    match ascent.mutation() {
        Mutation::Intact => {
            let live = ascent.path().get().inner.id;
            debug_assert_eq!(live, id);
            vec![]
        }
        Mutation::MaybeDropped => vec![log_destroyed(id)],
    }
}

fn rearm(child: &mut AndReturnHome) -> Vec<DemoEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

fn outer_handler(_ev: &KeyEvent, _ascent: &mut Ascent<OuterPath>) -> Vec<DemoEffect> {
    vec![DemoEffect::SetLayerHome]
}

fn inner_handler(_ev: &KeyEvent, ascent: &mut Ascent<InnerPath>) -> Vec<DemoEffect> {
    ascent.invalidate(2);
    vec![]
}
```

## Generated: Inner

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Inner {
    fn dispatch<'a, 'c>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> ::bind::Ascent<<Inner as ::bind::Place>::Path<'a>>
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

        let mut ascent = ::bind::Ascent::new(path);

        if let ::core::option::Option::Some(ev) = opt_0 {
            let out_effs =
                claim.with_exclusive(&mut ascent, |ascent| inner_handler(ev, ascent));
            ::core::iter::Extend::extend(effs, out_effs);
        }

        ascent
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
    fn dispatch<'a, 'c>(
        mut path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> ::bind::Ascent<<Outer as ::bind::Place>::Path<'a>>
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

        let ascent =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claim);

        let mut ascent = ascent.into_parent_ascent(effs, move |ascent| {
            let mut local =
                ::std::vec::Vec::<<M as ::bind::Bindings>::Effect>::new();

            if let ::core::option::Option::Some(id) = opt_0 {
                ::core::iter::Extend::extend(
                    &mut local,
                    ::bind::run_post(ascent, |ascent| after_child(id, ascent)),
                );
            }

            if opt_1 {
                ::core::iter::Extend::extend(
                    &mut local,
                    ::bind::run_post(ascent, |ascent| {
                        ::bind::only_if_intact(
                            |p: &mut <Outer as ::bind::Place>::Path<'a>| {
                                &mut p.get_mut().return_home
                            },
                            rearm,
                        )(ascent)
                    }),
                );
            }

            local
        });

        if let ::core::option::Option::Some(ev) = opt_2 {
            let e =
                claim.with_exclusive(&mut ascent, |ascent| outer_handler(ev, ascent));
            ::core::iter::Extend::extend(effs, e);
        }

        ascent
    }
}
```

## Walk: KeyA, Inner `invalidate(2)`

```text
dispatch entry
  claim_slot = None
  claim = Claim { slot: &mut claim_slot }

ASCENT Inner
  ascent = Ascent::new(path)                 // path is a field; counters 0
  claim.with_exclusive(&mut ascent, ...):
    try_take → Some(())
    ascent.invalidate(2)                     // inv=2; path still on ascent
  return ascent                              // path + inv=2 travel together

ASCENT Outer into_parent_ascent
  move path out of ascent (the PathMut held since descent)
  path.into_parent()                         // recover OuterPath — not a new mint
  hops = 1; inv stays 2
  rebuild Ascent { path: OuterPath, inv=2, hops=1 }
  run_posts:                                 // 1 < 2 → MaybeDropped
    after_child: uses pre-snapped id; path is still on ascent
    only_if_intact: skip
  return ascent

ASCENT Outer
  claim.with_exclusive → None
  outer_handler not run
```

## Walk: KeyB

```text
ASCENT Inner: Ascent::new(path); no exclusive
ASCENT Outer into_parent_ascent:
  hops=1, inv=0 → Intact
  rearm runs (path on ascent is live)
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Ascent<P>` (path + counters owned) + `Claim<'c>` + `into_parent_ascent` + `Claim::with_exclusive`

Path is a field of `Ascent`. Invalidate does not remove it. Free-function `dispatch` owns the claim slot.

### P4 — bind via `claim.with_exclusive`

### F1 — post

### F2 — invalidate(d)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape carrier

Reshape replaces the path field on `Ascent` with a new path value already in hand. Counters stay. Claim is uninvolved. No post-time mint from the live tree.

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post, including after invalidate.
3. `Ascent<P>` owns path and depth counters together. Path is not reconstructed at post time after a kill; it is the field already on `Ascent`.
4. `Claim<'c>` closes over the root claim slot only. Exclusive is `claim.with_exclusive(&mut ascent, body)`.
5. laserbeam `into_parent`: parent only; consumes the path value already held.
6. `into_parent_ascent` hops path, bumps hops, posts after the bump; claim is not in that type.
7. Kill: `ascent.invalidate(d)`; path remains on `Ascent`; later posts read `mutation()` against that same value.
8. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth` (live, not frozen).
9. Generate matches expand above.

## Tests

- `invalidate(2)`: after one framework hop MaybeDropped; after two Intact
- after invalidate, post still receives `Ascent` whose path field is the recovered path (not freshly minted)
- framework hop does not change `invalidation_depth`
- claim trap door is on `Claim`; path hops do not touch it
- KeyA / KeyB walks match above
- `into_parent_ascent` compiles with `PathMut<_, &mut Root>` while a live `Claim` exists in the same stack frame
