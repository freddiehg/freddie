# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

Two carriers. They close over different root slots. They never share a field.

```rust
// Top-level dispatch owns both slots:
//   let mut depth = Depth::new();
//   let mut claim_slot: Option<()> = None;
//   let mut claim = Claim::new(&mut claim_slot);

/// Hop counters. Owned at dispatch entry. Reborrowed by Ascent.
pub struct Depth {
    invalidation_depth: u32,
    ascent_hops: u32,
}

/// Path closes over `&mut Depth`. No claim here.
pub struct Ascent<'d, P> {
    path: P,
    depth: &'d mut Depth,
}

/// Something else closes over `&mut Option<()>` — the exclusive trap door.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}
```

Why this compiles:

- `P` may be `PathMut<N, Parent>` ending in `&mut Root`. Hopping is `path.into_parent()`, which consumes `path` by value. The interior `&mut` moves with it.
- `Ascent` only reborrows `Depth`. After the hop, rebuild `Ascent { path: parent, depth }` with the same `&mut Depth`. `invalidate` and the hop bump write through that reborrow; the next `mutation()` reads it.
- `Claim` is a separate value. Exclusive gating goes through `claim.with_exclusive(&mut ascent, body)`. Path surgery and claim never live in one struct, so their exclusive refs cannot be forced to alias.

`into_parent_ascent` only sees `Ascent`: move path → `into_parent` → bump hops on `depth` → rebuild `Ascent` with the same depth reborrow → run posts. Claim is not in that type.

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

Depth is live. After `invalidate(d)` or a hop bumps `ascent_hops`, the next `mutation()` sees the new numbers.

laserbeam `PathMut::into_parent` recovers parent only.

Kill: `ascent.invalidate(d)` with `d` = kill path hop count.

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

pub struct Depth {
    invalidation_depth: u32,
    ascent_hops: u32,
}

impl Depth {
    pub fn new() -> Self {
        Self {
            invalidation_depth: 0,
            ascent_hops: 0,
        }
    }

    fn mutation(&self) -> Mutation {
        if self.ascent_hops < self.invalidation_depth {
            Mutation::MaybeDropped
        } else {
            Mutation::Intact
        }
    }

    fn bump_ascent_hop(&mut self) {
        self.ascent_hops = self.ascent_hops.saturating_add(1);
    }

    fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }
}

/// Path closes over `&mut Depth`.
pub struct Ascent<'d, P> {
    path: P,
    depth: &'d mut Depth,
}

impl<'d, P> Ascent<'d, P> {
    pub fn new(path: P, depth: &'d mut Depth) -> Self {
        Self { path, depth }
    }

    pub fn path(&self) -> &P {
        &self.path
    }

    pub fn path_mut(&mut self) -> &mut P {
        &mut self.path
    }

    pub fn mutation(&self) -> Mutation {
        self.depth.mutation()
    }

    /// Kill coverage. d = path hops of the kill climb.
    pub fn invalidate(&mut self, d: u32) {
        self.depth.invalidate(d);
    }
}

impl<'d, Node, Parent> Ascent<'d, laserbeam::PathMut<Node, Parent>> {
    /// Framework hop: consume path → parent, bump hops on depth, run posts.
    /// Same `&mut Depth` reborrow; claim is not involved.
    pub fn into_parent_ascent<E>(
        self,
        sink: &mut Vec<E>,
        run_posts: impl FnOnce(&mut Ascent<'d, Parent>) -> Vec<E>,
    ) -> Ascent<'d, Parent> {
        let Ascent { path, depth } = self;
        let parent = path.into_parent();
        depth.bump_ascent_hop();
        let mut ascent = Ascent {
            path: parent,
            depth,
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

    /// Exclusive gate on the claim side; body gets the path/depth side.
    pub fn with_exclusive<'d, P, E>(
        &mut self,
        ascent: &mut Ascent<'d, P>,
        body: impl FnOnce(&mut Ascent<'d, P>) -> Vec<E>,
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
+1 hop:   hops=1 inv=2  →  1 < 2  MaybeDropped   // Outer posts here
+2 hops:  hops=2 inv=2  →  2 < 2  Intact
```

Order invariant inside `into_parent_ascent`: bump before posts. Framework never writes `invalidation_depth`; only `invalidate` does.

## User signatures

```rust
// pre — descent; no Ascent yet
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post — ascent; path closes over &mut Depth
fn post(pre_return: T, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
fn post(ascent: &mut Ascent<P>) -> Vec<M::Effect>;

// exclusive — body sees Ascent only; claim gate is Claim::with_exclusive
fn exclusive(ev: &SourceEvent, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
```

Path is `ascent.path()` / `ascent.path_mut()`. Mutation / invalidate are on `ascent`. Claim is never a method on `Ascent`.

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
pub fn run_post<'d, P, E>(
    ascent: &mut Ascent<'d, P>,
    body: impl FnOnce(&mut Ascent<'d, P>) -> Vec<E>,
) -> Vec<E> {
    body(ascent)
}

pub fn only_if_intact<'d, P, N, E>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<E>,
) -> impl FnOnce(&mut Ascent<'d, P>) -> Vec<E> {
    move |ascent| match ascent.mutation() {
        Mutation::Intact => f(project(ascent.path_mut())),
        Mutation::MaybeDropped => Vec::new(),
    }
}
```

## Dispatch

Root owns `Depth` and the claim slot. `Ascent` reborrows depth. `Claim` reborrows the slot. Both are threaded down; only `Ascent` is returned up the spine.

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a, 'd, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        depth: &'d mut Depth,
        claim: &mut Claim<'c>,
    ) -> Ascent<'d, Self::Path<'a>>
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
    let mut depth = Depth::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _ascent =
        <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut depth, &mut claim);
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
  ascent = Child::dispatch(child_path, event, effs, depth, claim)
  ascent = ascent.into_parent_ascent(effs, |ascent| { /* posts */ })

if leaf:
  ascent = Ascent::new(path, depth)

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
    fn dispatch<'a, 'd, 'c>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        depth: &'d mut ::bind::Depth,
        claim: &mut ::bind::Claim<'c>,
    ) -> ::bind::Ascent<'d, <Inner as ::bind::Place>::Path<'a>>
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

        let mut ascent = ::bind::Ascent::new(path, depth);

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
    fn dispatch<'a, 'd, 'c>(
        mut path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        depth: &'d mut ::bind::Depth,
        claim: &mut ::bind::Claim<'c>,
    ) -> ::bind::Ascent<'d, <Outer as ::bind::Place>::Path<'a>>
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

        let ascent = <Inner as ::bind::Dispatch<M>>::dispatch(
            inner_path, event, effs, depth, claim,
        );

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
  depth = Depth { inv: 0, hops: 0 }
  claim_slot = None
  claim = Claim { slot: &mut claim_slot }

ASCENT Inner
  ascent = Ascent::new(path, &mut depth)     // path closes over &mut depth
  claim.with_exclusive(&mut ascent, ...):
    try_take → Some(())                      // claim side only
    ascent.invalidate(2)                     // writes depth through ascent
  return ascent                              // still closes over &mut depth

ASCENT Outer into_parent_ascent              // claim not in this type
  move path out
  path.into_parent()
  depth.bump_ascent_hop → hops=1             // same Depth cell
  rebuild Ascent { parent, depth }
  run_posts:                                 // 1 < 2 → MaybeDropped
    after_child: MaybeDropped → LogDestroyed
    only_if_intact: skip
  return ascent

ASCENT Outer
  claim.with_exclusive(&mut ascent, ...):
    try_take → None                          // slot already Some
  outer_handler not run
```

## Walk: KeyB

```text
ASCENT Inner: Ascent::new(path, depth); no exclusive
ASCENT Outer into_parent_ascent:
  hops=1, inv=0 → Intact
  rearm runs
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Depth` + `Ascent<'d, P>` + `Claim<'c>` + `into_parent_ascent` + `Claim::with_exclusive`

Path closes over `&mut Depth`. Claim closes over `&mut Option<()>`. Free-function `dispatch` owns both slots.

### P4 — bind via `claim.with_exclusive`

### F1 — post

### F2 — invalidate(d)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape carrier

Reshape hands a new path value. Reconstruct `Ascent { path: new_path, depth }` with the same depth reborrow. Claim is untouched and uninvolved.

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. Root owns `Depth` and `claim_slot`. `Ascent<'d, P>` closes over `&mut Depth` only. `Claim<'c>` closes over `&mut Option<()>` only.
4. laserbeam `into_parent`: parent only; consumes path by value.
5. `into_parent_ascent` is path+depth only: hop path, bump hops on depth, posts after the bump.
6. Kill: `ascent.invalidate(d)`; later hops/posts read the new depth through the same cell.
7. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth` (live, not frozen).
8. Exclusive is `claim.with_exclusive(&mut ascent, body)`. Trap door is on `Claim`, not on `Ascent`.
9. Generate matches expand above.

## Tests

- `invalidate(2)`: after one framework hop MaybeDropped; after two Intact
- framework hop does not change `invalidation_depth`
- claim trap door is on `Claim`; path hops do not touch it
- KeyA / KeyB walks match above
- `into_parent_ascent` compiles with `PathMut<_, &mut Root>` while a live `Claim` exists in the same stack frame
