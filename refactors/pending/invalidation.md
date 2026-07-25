# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

**Path and context are one value. User code gets `&mut Ascent<P>`.**

```rust
pub struct Ascent<P> {
    path: P,
    state: AscentState,
}
```

Dispatch threads `Ascent<P>` by value across hops. Handlers receive **`&mut Ascent<P>`** (path + state). They do not get bare `AscentState`, and they do not get a separate path argument.

**`AscentState` (private fields, only reached through `Ascent` methods):**

- **`invalidation_depth`** — kill coverage. Set by `ascent.invalidate(d)` only.
- **`ascent_hops`** — framework parent recoveries since leaf. Bumped only inside `into_parent_ascent`.
- **`claim`** — `Option<()>`, one-way trap door via `ascent.claim()`.

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

**laserbeam `PathMut::into_parent`** recovers parent only.

**`Ascent::into_parent_ascent`:** recover parent → bump `ascent_hops` → `run_posts(&mut Ascent<Parent>)`.

**Kill:** `ascent.invalidate(d)` with d = kill path hop count.

User-facing methods on `&mut Ascent` / `&Ascent` (framework-only methods stay private on `AscentState`):

| method | role |
|---|---|
| `path` / `path_mut` | all |
| `mutation` / `claimed` | all |
| `claim` | exclusive (also used by `with_exclusive` before body) |
| `invalidate` | exclusive kill |
| `bump_ascent_hop` | private / only `into_parent_ascent` |

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

struct AscentState {
    invalidation_depth: u32,
    ascent_hops: u32,
    claim: Option<()>,
}

impl AscentState {
    fn new() -> Self {
        Self {
            invalidation_depth: 0,
            ascent_hops: 0,
            claim: None,
        }
    }

    fn mutation(&self) -> Mutation {
        if self.ascent_hops < self.invalidation_depth {
            Mutation::MaybeDropped
        } else {
            Mutation::Intact
        }
    }

    fn claimed(&self) -> bool {
        self.claim.is_some()
    }

    fn claim(&mut self) -> Option<()> {
        if self.claim.is_some() {
            None
        } else {
            self.claim = Some(());
            Some(())
        }
    }

    fn bump_ascent_hop(&mut self) {
        self.ascent_hops = self.ascent_hops.saturating_add(1);
    }

    fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }
}

pub struct Ascent<P> {
    path: P,
    state: AscentState,
}

impl<P> Ascent<P> {
    pub fn new(path: P) -> Self {
        Self {
            path,
            state: AscentState::new(),
        }
    }

    pub fn path(&self) -> &P {
        &self.path
    }

    pub fn path_mut(&mut self) -> &mut P {
        &mut self.path
    }

    pub fn mutation(&self) -> Mutation {
        self.state.mutation()
    }

    pub fn claimed(&self) -> bool {
        self.state.claimed()
    }

    /// One-way trap door. Some(()) if this call took it.
    pub fn claim(&mut self) -> Option<()> {
        self.state.claim()
    }

    /// Kill coverage. d = path hops of the kill climb.
    pub fn invalidate(&mut self, d: u32) {
        self.state.invalidate(d);
    }

    /// Exclusive gate then body with `&mut self`.
    pub fn with_exclusive<E>(
        &mut self,
        body: impl FnOnce(&mut Ascent<P>) -> Vec<E>,
    ) -> Vec<E> {
        match self.claim() {
            None => Vec::new(),
            Some(()) => body(self),
        }
    }
}

impl<Node, Parent> Ascent<laserbeam::PathMut<Node, Parent>> {
    /// Framework hop: recover parent, bump ascent_hops, run posts on `&mut Ascent<Parent>`.
    pub fn into_parent_ascent<E>(
        self,
        sink: &mut Vec<E>,
        run_posts: impl FnOnce(&mut Ascent<Parent>) -> Vec<E>,
    ) -> Ascent<Parent> {
        let Ascent { path, mut state } = self;
        let parent = path.into_parent();
        state.bump_ascent_hop();
        let mut ascent = Ascent {
            path: parent,
            state,
        };
        let post_effs = run_posts(&mut ascent);
        sink.extend(post_effs);
        ascent
    }
}
```

## User signatures

```rust
// pre — descent; no Ascent yet
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post — ascent; &mut Ascent is path + state
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
    let ascent = <N as Dispatch<M>>::dispatch(path, event, &mut effs);
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
  ascent = Child::dispatch(child_path, event, effs)
  ascent = ascent.into_parent_ascent(effs, |ascent| { /* posts */ })

if leaf:
  ascent = Ascent::new(path)

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
    fn dispatch<'a>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
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
    fn dispatch<'a>(
        mut path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
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
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs);

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
ASCENT Inner
  ascent = Ascent::new(path)
  with_exclusive(&mut ascent):
    claim → Some(())
    ascent.invalidate(2)
  return ascent

ASCENT Outer into_parent_ascent
  path.into_parent()
  bump_ascent_hop → hops=1
  run_posts(&mut Ascent<OuterPath>):
    after_child: mutation MaybeDropped → LogDestroyed
    only_if_intact: skip
  return ascent

ASCENT Outer with_exclusive
  claim → None
  outer_handler not run
```

## Walk: KeyB

```text
ASCENT Inner: Ascent::new; no exclusive
ASCENT Outer into_parent_ascent:
  hops=1, inv=0 → Intact
  rearm runs
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Ascent<P>` + `into_parent_ascent` + `with_exclusive`

Handlers take `&mut Ascent<P>`.

### P4 — bind via `with_exclusive`

### F1 — post

### F2 — invalidate(d)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape carrier

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. Thread `Ascent<P>`. User code gets `&mut Ascent<P>`.
4. laserbeam `into_parent`: parent only.
5. `into_parent_ascent` bumps `ascent_hops` internally.
6. Kill: `ascent.invalidate(d)`.
7. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth`.
8. `claim()` is `Option<()>` trap door.
9. Generate matches expand above.

## Tests

- `invalidate(2)`: after one framework hop MaybeDropped; after two Intact
- framework hop does not change `invalidation_depth`
- claim trap door
- KeyA / KeyB walks match above
