# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

User code gets `&mut Ascent<'c, P>`. Path and hop counters are fields on that value. Claim is not: the top-level dispatch owns the claim slot, and every `Ascent` only reborrows it.

```rust
// Top-level dispatch owns the slot:
//   let mut claim: Option<()> = None;
//
// Ascent reborrows it for the whole spine.
pub struct Ascent<'c, P> {
    path: P,
    invalidation_depth: u32,
    ascent_hops: u32,
    claim: &'c mut Option<()>,
}
```

Why this shape compiles:

- `P` may be `PathMut<N, Parent>` ending in `&mut Root`. Hopping is `path.into_parent()`, which consumes `path` by value and returns `Parent`. The interior `&mut` moves with it. Nothing else may be borrowing that path across the hop.
- Hop counters are owned `u32`s on `Ascent`. They move with the reconstruct after `into_parent`. They are not a `&mut` into a shared bag that also holds claim. `invalidate` / `bump` write those fields; the next `mutation()` read sees the new numbers.
- Claim lives in a separate stack slot at the root of `dispatch`. `Ascent` carries `&'c mut Option<()>`. Path's interior `&mut Root` and the claim reborrow never alias: two different places.

So `into_parent_ascent` is: move path out → `into_parent` → bump `ascent_hops` → rebuild `Ascent<'c, Parent>` with the same claim reborrow → run posts on that.

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

Depth is live. After `invalidate(d)` or a hop bumps `ascent_hops`, the next `mutation()` (and the next framework decision that uses it) sees the new numbers.

laserbeam `PathMut::into_parent` recovers parent only (path-only; no claim, no counters).

Kill: `ascent.invalidate(d)` with `d` = kill path hop count.

Methods:

- `path` / `path_mut` — all
- `mutation` / `claimed` — all
- `claim` — exclusive (also used by `with_exclusive` before body)
- `invalidate` — exclusive kill
- hop bump — private; only `into_parent_ascent`

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

pub struct Ascent<'c, P> {
    path: P,
    invalidation_depth: u32,
    ascent_hops: u32,
    claim: &'c mut Option<()>,
}

impl<'c, P> Ascent<'c, P> {
    pub fn new(path: P, claim: &'c mut Option<()>) -> Self {
        Self {
            path,
            invalidation_depth: 0,
            ascent_hops: 0,
            claim,
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

    pub fn claimed(&self) -> bool {
        self.claim.is_some()
    }

    /// One-way trap door. Some(()) if this call took it.
    pub fn claim(&mut self) -> Option<()> {
        if self.claim.is_some() {
            None
        } else {
            *self.claim = Some(());
            Some(())
        }
    }

    /// Kill coverage. d = path hops of the kill climb.
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Exclusive gate then body with `&mut self`.
    pub fn with_exclusive<E>(
        &mut self,
        body: impl FnOnce(&mut Ascent<'c, P>) -> Vec<E>,
    ) -> Vec<E> {
        match self.claim() {
            None => Vec::new(),
            Some(()) => body(self),
        }
    }
}

impl<'c, Node, Parent> Ascent<'c, laserbeam::PathMut<Node, Parent>> {
    /// Framework hop: consume path → parent, bump ascent_hops, run posts.
    /// Claim reborrow is the same `'c` slot; counters move by value.
    pub fn into_parent_ascent<E>(
        self,
        sink: &mut Vec<E>,
        run_posts: impl FnOnce(&mut Ascent<'c, Parent>) -> Vec<E>,
    ) -> Ascent<'c, Parent> {
        let Ascent {
            path,
            invalidation_depth,
            mut ascent_hops,
            claim,
        } = self;
        let parent = path.into_parent();
        ascent_hops = ascent_hops.saturating_add(1);
        let mut ascent = Ascent {
            path: parent,
            invalidation_depth,
            ascent_hops,
            claim,
        };
        let post_effs = run_posts(&mut ascent);
        sink.extend(post_effs);
        ascent
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

// post — ascent; &mut Ascent is path + counters + claim reborrow
fn post(pre_return: T, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
fn post(ascent: &mut Ascent<P>) -> Vec<M::Effect>;

// exclusive — same; claim is applied by with_exclusive before body
fn exclusive(ev: &SourceEvent, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
```

Path is `ascent.path()` / `ascent.path_mut()`. Mutation / claim / invalidate are methods on `ascent`.

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
pub fn run_post<'c, P, E>(
    ascent: &mut Ascent<'c, P>,
    body: impl FnOnce(&mut Ascent<'c, P>) -> Vec<E>,
) -> Vec<E> {
    body(ascent)
}

pub fn only_if_intact<'c, P, N, E>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<E>,
) -> impl FnOnce(&mut Ascent<'c, P>) -> Vec<E> {
    move |ascent| match ascent.mutation() {
        Mutation::Intact => f(project(ascent.path_mut())),
        Mutation::MaybeDropped => Vec::new(),
    }
}
```

## Dispatch

Claim slot is owned at the free-function entry. Every `Dispatch::dispatch` takes a reborrow and returns `Ascent` still holding that reborrow.

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
        claim: &'c mut Option<()>,
    ) -> Ascent<'c, Self::Path<'a>>
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
    let mut claim = None;
    let ascent = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    if ascent.claimed() || !effs.is_empty() {
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
  ascent = Ascent::new(path, claim)

if exclusive scheduled:
  effs2 = ascent.with_exclusive(|ascent| handler(ev, ascent))
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
        claim: &'c mut ::core::option::Option<()>,
    ) -> ::bind::Ascent<'c, <Inner as ::bind::Place>::Path<'a>>
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

        let mut ascent = ::bind::Ascent::new(path, claim);

        if let ::core::option::Option::Some(ev) = opt_0 {
            let out_effs = ascent.with_exclusive(|ascent| inner_handler(ev, ascent));
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
        claim: &'c mut ::core::option::Option<()>,
    ) -> ::bind::Ascent<'c, <Outer as ::bind::Place>::Path<'a>>
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
            let e = ascent.with_exclusive(|ascent| outer_handler(ev, ascent));
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

ASCENT Inner
  ascent = Ascent::new(path, &mut claim_slot)
  with_exclusive(&mut ascent):
    claim → Some(())                 // writes claim_slot through reborrow
    ascent.invalidate(2)             // inv=2 on this Ascent; hops still 0
  return ascent                      // still holds &mut claim_slot

ASCENT Outer into_parent_ascent
  move path out of ascent
  path.into_parent()                 // Parent recovered; interior &mut moves
  ascent_hops = 1                    // counters by value; inv still 2
  rebuild Ascent { parent, inv=2, hops=1, claim: same reborrow }
  run_posts:                         // mutation() is 1 < 2 → MaybeDropped
    after_child: MaybeDropped → LogDestroyed
    only_if_intact: skip
  return ascent

ASCENT Outer with_exclusive
  claim → None                       // slot already Some
  outer_handler not run
```

## Walk: KeyB

```text
ASCENT Inner: Ascent::new(path, claim); no exclusive
ASCENT Outer into_parent_ascent:
  hops=1, inv=0 → Intact
  rearm runs
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Ascent<'c, P>` + root claim slot + `into_parent_ascent` + `with_exclusive`

Handlers take `&mut Ascent<'c, P>`. Free-function `dispatch` owns `Option<()>` and passes `&mut` down.

### P4 — bind via `with_exclusive`

### F1 — post

### F2 — invalidate(d)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape carrier

Reshape hands a new path value. Reconstruct `Ascent` with that path, same counters, same claim reborrow. Path may again contain `&mut`; claim still does not alias it.

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. Top-level `dispatch` owns `claim: Option<()>`. `Ascent<'c, P>` reborrows it. Counters are owned on `Ascent`. Path is owned on `Ascent`.
4. laserbeam `into_parent`: parent only; consumes path by value.
5. `into_parent_ascent` destructures, moves path through `into_parent`, bumps `ascent_hops`, rebuilds with the same claim reborrow; posts run after the bump.
6. Kill: `ascent.invalidate(d)`; later hops/posts read the new depth.
7. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth` (live, not frozen).
8. `claim()` is `Option<()>` trap door on the root slot.
9. Generate matches expand above.

## Tests

- `invalidate(2)`: after one framework hop MaybeDropped; after two Intact
- framework hop does not change `invalidation_depth`
- claim trap door; root slot is what every level sees
- KeyA / KeyB walks match above
- into_parent_ascent compiles with `PathMut<_, &mut Root>` (path interior mut + claim reborrow coexist)
