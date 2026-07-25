# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

**Path and ascent context are one value: `Ascent<P>`.**

```rust
pub struct Ascent<P> {
    path: P,
    state: AscentState,
}
```

Dispatch threads `Ascent<P>`. Hop counting is ownership-based: recovering a parent yields a **`Complete`** that must be consumed to update counters. No interior mutability; no runtime borrow checker.

**`Complete`** — sealed proof one `into_parent` hop happened. Only produced by `Ascent::into_parent`. Only consumed by:

- `complete_ascent_hop` — framework hop (bumps `ascent_hops`)
- `complete_kill_hop` — kill climb (raises `invalidation_depth` by one, via max with running kill hops)

Dropping `Complete` without consuming it is a bug (`#[must_use]`).

**`AscentState` (inside `Ascent`, not bare to user code):**

- **`invalidation_depth`** — kill coverage. Only via consumed kill `Complete`s / `invalidate`.
- **`ascent_hops`** — framework hops since leaf. Only via consumed ascent `Complete`s.
- **`claim`** — `Option<()>`, one-way trap door.

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

**laserbeam `PathMut::into_parent`** recovers parent path only.

**Framework hop:** `into_parent` → `Complete` → `complete_ascent_hop` → posts → `Ascent<Parent>`.

**Kill climb:** each path `into_parent` → `Complete` → `complete_kill_hop` (or batch into `invalidate(n)` after n completes).

User handlers get **capability views**:

| | `PostCtx` | `ExclusiveCtx` |
|---|---|---|
| `mutation()` | yes | yes |
| `claimed()` | yes | yes |
| `claim()` | no | yes |
| `invalidate` / kill completes | no | yes |

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

pub struct AscentState {
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

    /// Framework hop: must consume `Complete` from `Ascent::into_parent`.
    fn complete_ascent_hop(&mut self, _hop: Complete) {
        self.ascent_hops = self.ascent_hops.saturating_add(1);
    }

    /// Kill hop: each consumed `Complete` counts one unit of kill coverage.
    fn complete_kill_hop(&mut self, _hop: Complete) {
        let next = self.invalidation_depth.saturating_add(1);
        self.invalidation_depth = self.invalidation_depth.max(next);
        // equivalent: invalidation_depth += 1 when starting from 0 per-kill;
        // max keeps concurrent kills monotone.
    }

    /// Record kill coverage of exactly `d` hops (after d kill completes, or directly).
    fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }
}

/// Proof one parent recovery happened. Only from `Ascent::into_parent`.
/// Must be fed to `complete_ascent_hop` or `complete_kill_hop` (or equivalent).
#[must_use]
pub struct Complete {
    _seal: (),
}

/// Path + state. Dispatch threads this.
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

    pub fn claimed(&self) -> bool {
        self.state.claimed()
    }

    pub fn post_ctx(&mut self) -> PostCtx<'_> {
        PostCtx {
            state: &mut self.state,
        }
    }

    pub fn with_exclusive<E>(
        self,
        body: impl FnOnce(Node<P, ()>, ExclusiveCtx<'_>) -> (Vec<E>, P),
    ) -> (Ascent<P>, Vec<E>) {
        let Ascent { path, mut state } = self;
        let mut ctx = ExclusiveCtx {
            state: &mut state,
        };
        match ctx.claim() {
            None => (Ascent { path, state }, Vec::new()),
            Some(()) => {
                let (path, effs) = body(
                    Node {
                        parent: path,
                        data: (),
                    },
                    ctx,
                );
                (Ascent { path, state }, effs)
            }
        }
    }
}

impl<Node, Parent> Ascent<laserbeam::PathMut<Node, Parent>> {
    /// Recover parent path. Returns `Complete` that **must** be used to count the hop.
    pub fn into_parent(self) -> (Ascent<Parent>, Complete) {
        let Ascent { path, state } = self;
        let parent = path.into_parent();
        (
            Ascent {
                path: parent,
                state,
            },
            Complete { _seal: () },
        )
    }

    /// Framework hop: into_parent → complete_ascent_hop → posts.
    /// Does not change invalidation_depth.
    pub fn into_parent_ascent<E>(
        self,
        sink: &mut Vec<E>,
        run_posts: impl FnOnce(Parent, PostCtx<'_>) -> (Parent, Vec<E>),
    ) -> Ascent<Parent> {
        let (mut ascent, hop) = self.into_parent();
        ascent.state.complete_ascent_hop(hop);
        let Ascent { path, mut state } = ascent;
        let (path, post_effs) = run_posts(
            path,
            PostCtx {
                state: &mut state,
            },
        );
        sink.extend(post_effs);
        Ascent { path, state }
    }
}

pub struct PostCtx<'a> {
    state: &'a mut AscentState,
}

impl<'a> PostCtx<'a> {
    pub fn mutation(&self) -> Mutation {
        self.state.mutation()
    }

    pub fn claimed(&self) -> bool {
        self.state.claimed()
    }
}

pub struct ExclusiveCtx<'a> {
    state: &'a mut AscentState,
}

impl<'a> ExclusiveCtx<'a> {
    pub fn mutation(&self) -> Mutation {
        self.state.mutation()
    }

    pub fn claimed(&self) -> bool {
        self.state.claimed()
    }

    pub fn claim(&mut self) -> Option<()> {
        self.state.claim()
    }

    /// Record kill coverage of `d` hops (after climbing d times and completing each hop).
    pub fn invalidate(&mut self, d: u32) {
        self.state.invalidate(d);
    }

    /// One kill hop: consume `Complete` from `Ascent::into_parent` during a kill climb.
    pub fn complete_kill_hop(&mut self, hop: Complete) {
        self.state.complete_kill_hop(hop);
    }
}
```

## User signatures

```rust
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

fn post(
    pre_return: T,
    node: Node<P, D>,
    ctx: PostCtx<'_>,
) -> (Vec<M::Effect>, P);

fn post(
    node: Node<P, D>,
    ctx: PostCtx<'_>,
) -> (Vec<M::Effect>, P);

fn exclusive(
    ev: &SourceEvent,
    node: Node<P, D>,
    ctx: ExclusiveCtx<'_>,
) -> (Vec<M::Effect>, P);
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

    /// Parent only. No AscentState. No hop counters.
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

## Helpers (bind)

```rust
pub fn run_post<P, E>(
    path: P,
    ctx: PostCtx<'_>,
    body: impl FnOnce(Node<P, ()>, PostCtx<'_>) -> (Vec<E>, P),
) -> (P, Vec<E>) {
    body(
        Node {
            parent: path,
            data: (),
        },
        ctx,
    )
}

pub fn only_if_intact<P, N, E>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<E>,
) -> impl FnOnce(Node<P, ()>, PostCtx<'_>) -> (Vec<E>, P) {
    move |mut node, ctx| {
        let effects = match ctx.mutation() {
            Mutation::Intact => f(project(&mut node.parent)),
            Mutation::MaybeDropped => Vec::new(),
        };
        (effects, node.parent)
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
  ascent = Child::dispatch(child_path, event, effs)   // Ascent<PathMut<...>>
  ascent = ascent.into_parent_ascent(effs, |parent, post_ctx| { posts... })
  // path.into_parent(); bump_ascent_hop(); run_posts;  NOT invalidate

if leaf:
  ascent = Ascent::new(path)

if exclusive scheduled:
  (ascent, effs2) = ascent.with_exclusive(|node, excl_ctx| handler(ev, node, excl_ctx))
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

fn after_child(
    id: ChildId,
    node: Node<OuterPath, ()>,
    ctx: PostCtx<'_>,
) -> (Vec<DemoEffect>, OuterPath) {
    match ctx.mutation() {
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
    _ctx: ExclusiveCtx<'_>,
) -> (Vec<DemoEffect>, OuterPath) {
    (vec![DemoEffect::SetLayerHome], node.parent)
}

fn inner_handler(
    _ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    mut ctx: ExclusiveCtx<'_>,
) -> (Vec<DemoEffect>, InnerPath) {
    // Kill coverage of two hops. Either:
    //   climb with Ascent::into_parent twice, complete_kill_hop each Complete, or
    //   invalidate(2) once the hop count is known.
    ctx.invalidate(2);
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
            let (ascent, out_effs) = ascent.with_exclusive(|node, ctx| {
                inner_handler(ev, node, ctx)
            });
            ::core::iter::Extend::extend(effs, out_effs);
            return ascent;
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

        let mut ascent = ascent.into_parent_ascent(effs, move |parent, ctx| {
            let mut local =
                ::std::vec::Vec::<<M as ::bind::Bindings>::Effect>::new();
            let mut path = parent;

            if let ::core::option::Option::Some(id) = opt_0 {
                let (p, e) = ::bind::run_post(path, ctx, |node, ctx| {
                    after_child(id, node, ctx)
                });
                path = p;
                ::core::iter::Extend::extend(&mut local, e);
            }

            if opt_1 {
                let (p, e) = ::bind::run_post(path, ctx, |node, ctx| {
                    ::bind::only_if_intact(
                        |p: &mut <Outer as ::bind::Place>::Path<'a>| {
                            &mut p.get_mut().return_home
                        },
                        rearm,
                    )(node, ctx)
                });
                path = p;
                ::core::iter::Extend::extend(&mut local, e);
            }

            (path, local)
        });

        if let ::core::option::Option::Some(ev) = opt_2 {
            let (ascent2, e) = ascent.with_exclusive(|node, ctx| {
                outer_handler(ev, node, ctx)
            });
            ascent = ascent2;
            ::core::iter::Extend::extend(effs, e);
        }

        ascent
    }
}
```

## Walk: KeyA, Inner `invalidate(2)`

```text
DESCENT Outer / Inner as usual

ASCENT Inner
  ascent = Ascent::new(path)             // inv=0, hops=0
  with_exclusive:
    claim → Some(())
    invalidate(2)                        // inv=2
  return Ascent { InnerPath, state }

ASCENT Outer into_parent_ascent
  (ascent, hop) = into_parent()          // Complete must be returned
  complete_ascent_hop(hop)               // hops = 1; inv still 2
  posts PostCtx:
    mutation: 1 < 2 → MaybeDropped
    after_child → LogDestroyed
    only_if_intact → skip
  return Ascent { OuterPath, state }     // inv=2, hops=1

ASCENT Outer with_exclusive
  claim → None
  outer_handler not run

// next hop: bump hops=2; posts see 2 < 2 → Intact
```

## Walk: KeyB

```text
ASCENT Inner: Ascent::new; no exclusive
ASCENT Outer into_parent_ascent:
  bump hops=1
  inv=0 → Intact
  rearm runs
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink

### P2 — from_fn framework-only

### P3 — `Ascent<P>` + PostCtx + ExclusiveCtx + `into_parent_ascent`

Dispatch returns `Ascent<Path>`. Framework bumps `ascent_hops` only. Kill uses `invalidate`.

### P4 — bind via `with_exclusive`

### F1 — post + PostCtx

### F2 — invalidate(d)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape: kill path hops feed `invalidate(d)`

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. Thread `Ascent<P>` (path + state together).
4. User code: `PostCtx` / `ExclusiveCtx` only.
5. laserbeam `into_parent`: parent only.
6. `Ascent::into_parent` returns `Complete`; hop counts only when `Complete` is consumed (`complete_ascent_hop` / `complete_kill_hop` / `invalidate`).
7. No interior mutability; ownership of `Complete` is the check.
8. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth`.
9. `claim()` is `Option<()>` trap door.
10. Generate matches expand above.

## Tests

- `invalidate(2)`: after one framework hop MaybeDropped; after two Intact
- framework hop does not change `invalidation_depth`
- user code cannot call `bump_ascent_hop` / raw `AscentState` methods
- claim trap door
- KeyA / KeyB walks match above
