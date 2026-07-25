# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

Spine `A → B → C`. Descent builds one path all the way to `C`:

```text
PathMut<C, PathMut<B, A>>
```

Each `into_parent` peels one owned layer. Two hops and the only path you still own is `A`. You do not still hold a path to `B` or to `C`. Those values were moved out by `into_parent` and are gone.

B's post still runs (it was scheduled). It does not receive a path to B. It receives the path you still own (A) plus the fact that the peeled segment may have been killed. That pair is what "potentially invalidated" is for: you have to pass it, because the live path alone is only A.

```text
at C:  path = PathMut<C, PathMut<B, A>>
       exclusive may invalidate (coverage over some prefix of the spine)

into_parent → path = PathMut<B, A>     // C ownership gone
into_parent → path = A                 // B ownership gone

B's post: live path is A only.
          must also be told about the B segment: Intact vs MaybeDropped
          (and any pre-snapped data from descent — ChildId, etc.)
```

Ideal meaning of `invalidate`: that section of the path is no longer owned. `into_parent` is the only move; you cannot get that segment back. No re-projection down from A into a "B" that might be a different value after the kill.

Stopgap (acceptable for now): after `into_parent`, the framework still hands posts a `PathMut` at the level that just became current (B after one hop from C). That is not recovering ownership of a killed segment from nothing; it is keeping the parent half of the path that `into_parent` returned. After a kill, that path is only usable as potentially invalidated: `get` re-derives from the live tree, so it may not be the same B, and the doc does not pretend it is. `mutation()` is how posts are told. Posts that need identity after a kill use pre-snapped descent data, not "the path is still the old node."

Claim is a separate carrier (exclusive trap door). It is not part of the path/invalidation value.

```rust
// Live path you still own, plus hop coverage for peeled/killed segments.
pub struct Ascent<P> {
    path: P,
    invalidation_depth: u32,
    ascent_hops: u32,
}

// Exclusive only. Root owns the slot; this reborrows it.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}
```

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

`ascent_hops` = how many `into_parent` recoveries since the leaf. `invalidation_depth` = kill coverage set by `invalidate(d)` only. Live: the next `mutation()` after either changes sees the new numbers.

Framework hop (`into_parent_ascent`): consume path → parent, bump `ascent_hops`, run this level's posts on `Ascent<Parent>`, return that `Ascent`. Claim is not in this type.

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

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

    /// Kill coverage. d = how many hops of path from the leaf are covered.
    /// Does not by itself drop the path value; ownership loss is into_parent.
    pub fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }
}

impl<Node, Parent> Ascent<laserbeam::PathMut<Node, Parent>> {
    /// Peel one path layer, bump hops, run posts at the parent path.
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

    pub fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }

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

Hop accounting, leaf called `invalidate(2)`:

```text
leaf C:   hops=0 inv=2  →  MaybeDropped
after 1 into_parent (path is B):  hops=1 inv=2  →  MaybeDropped   // B's posts
after 2 into_parent (path is A):  hops=2 inv=2  →  Intact           // A's posts
```

Bump before posts. Framework never writes `invalidation_depth`.

Stopgap note: at "path is B", B's posts receive `Ascent<PathMut<B, A>>` — the parent half returned by `into_parent` from C, not a new mint from the live tree at post time. After kill, treat it as potentially invalidated (`mutation()`), not as proof it is the same B.

## User signatures

```rust
// pre — descent; path still owned down the spine; shared borrow only
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post — scheduled set is final; runs even when mutation is MaybeDropped.
// `ascent.path()` is the path still owned at this hop (parent after into_parent).
// It is not a re-created path into a killed child. Pre-snapped T carries identity
// that must survive kill; do not assume path.get() is the pre-kill node when MaybeDropped.
fn post(pre_return: T, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
fn post(ascent: &mut Ascent<P>) -> Vec<M::Effect>;

// exclusive — claim gate is Claim::with_exclusive; body sees Ascent
fn exclusive(ev: &SourceEvent, ascent: &mut Ascent<P>) -> Vec<M::Effect>;
```

## PathMut (laserbeam)

Unchanged. `into_parent` consumes and returns parent only. `get` / `get_mut` re-derive from the parent each time (not a frozen node snapshot).

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

/// Path mutation only when this hop is still Intact.
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
DESCENT: schedule opts (pre runs here; may snap identity)

if child:
  child_path = PathMut::from_fn(path, proj_mut, proj_ref)   // own path one level deeper
  ascent = Child::dispatch(child_path, event, effs, claim)
  // peel child path; posts at this level see parent path + mutation from hops/inv
  ascent = ascent.into_parent_ascent(effs, |ascent| { /* this level's posts */ })

if leaf:
  ascent = Ascent::new(path)

if exclusive scheduled:
  claim.with_exclusive(&mut ascent, |ascent| handler(ev, ascent))

return ascent
```

## DX example

A = root path, B = Outer, C = Inner.

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

// B's post: ascent.path is OuterPath (parent after into_parent from Inner).
// When MaybeDropped, do not treat path.get() as the pre-kill child; use snapped id.
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

        // Peel Inner → path is Outer (B). B's posts run here with Ascent<OuterPath>
        // and mutation from hops/inv. Not a path re-minted into a killed C.
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
path at C = PathMut<Inner, OuterPath>

ASCENT Inner (C)
  Ascent::new(path_to_C)
  claim.with_exclusive: take claim, invalidate(2)
  return Ascent { path_to_C, inv=2, hops=0 }

ASCENT Outer into_parent_ascent
  into_parent → path_to_B (OuterPath); C layer ownership gone
  hops=1
  B posts on Ascent { path_to_B, inv=2, hops=1 } → MaybeDropped
    after_child: snapped id, not path.get() as pre-kill proof
    only_if_intact: skip
  return that Ascent

  (further into_parent would yield A only; B layer ownership gone)

ASCENT Outer exclusive: claim already taken → skip
```

## Walk: KeyB

```text
no invalidate; hops=1 inv=0 → Intact at B posts
rearm may use path
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Ascent<P>` + `Claim<'c>` + `into_parent_ascent` + `Claim::with_exclusive`

Path ownership peels with `into_parent`. Posts get the path still owned at that hop plus `mutation()`.

### P4 — bind via `claim.with_exclusive`

### F1 — post

### F2 — invalidate(d)

### F3 — pre_post (snap identity before the child can die)

### F4 — only_if_intact + rearm

### F5 — reshape carrier

Ideal later: invalidate drops ownership of the covered segment for real (no path value for that segment remains). Stopgap: keep parent half after `into_parent`, flag MaybeDropped; sameness after kill is unknown.

## Rules

1. Descent schedules; set final. Ascent runs every scheduled post.
2. Path is owned and nested. `into_parent` peels one layer; that layer is no longer in the owned path.
3. After hops up to A, the owned path is only A. Intermediate posts do not still hold a path to C; they hold whatever parent half the last `into_parent` returned, plus `mutation()`.
4. Ideal invalidate: covered path section is unowned forever. Stopgap: parent half remains as `PathMut` but is only potentially invalidated; `get` re-derives and is not a sameness proof.
5. `Ascent<P>` = path still owned + hop counters. `Claim` is separate.
6. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth`.
7. Pre-snap for identity that must survive kill. `only_if_intact` for path mutation posts.
8. Generate matches expand above.

## Tests

- A→B→C: after two `into_parent`s owned path is A
- `invalidate(2)`: B posts see MaybeDropped; A posts see Intact
- after invalidate, MaybeDropped post uses pre-snap, not path identity
- `only_if_intact` skips when MaybeDropped
- claim trap door on `Claim`
- KeyA / KeyB walks match above
