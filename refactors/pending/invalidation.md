# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root.

## Model

**`AscentState`** is framework-owned. User code never receives bare `&mut AscentState`.

Handlers receive a **capability view** that holds `&mut AscentState` and exposes only the methods that role may use:

| | `PostCtx` | `ExclusiveCtx` |
|---|---|---|
| `mutation()` | yes | yes |
| `claimed()` | yes | yes |
| `claim()` | no | yes (trap door) |
| `invalidate(d)` | no | yes |
| hop counters | no | no |

**Depth is two numbers:**

- **`invalidation_depth`** — how many hops of spine a kill covers (from the exclusive’s level upward). **Only kill / path recovery mutates this** (`invalidate`). Framework post hops do **not** change it.
- **`ascent_hops`** — how many framework parent recoveries since the leaf. **Only framework `into_parent_ascent` bumps this** (once per hop, before posts at the parent).

```text
mutation() = if ascent_hops < invalidation_depth { MaybeDropped } else { Intact }
```

Example: leaf exclusive `invalidate(2)`. Levels from leaf: hop 0 = leaf, hop 1 = Outer, hop 2 = Root.

- After leave leaf, `ascent_hops = 1` → Outer posts: `1 < 2` → MaybeDropped
- After leave Outer, `ascent_hops = 2` → Root posts: `2 < 2` → Intact

**`into_parent` (laserbeam)** only recovers the parent path. It does **not** touch invalidation depth.

**Kill** is what raises `invalidation_depth` (via `ExclusiveCtx::invalidate`, driven by how many path hops the kill climbs — `into_parent().into_parent()` ⇒ `invalidate(2)`).

**`claim()`** is a one-way trap door: `Option<()>`.

## Types (`crates/bind`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Intact,
    MaybeDropped,
}

pub struct AscentState {
    /// Kill coverage from the exclusive’s level (inclusive hops upward).
    /// Mutated only by invalidate (kill / path climb), never by framework post hops.
    invalidation_depth: u32,
    /// Framework recoveries since leaf. Bumped only by into_parent_ascent.
    ascent_hops: u32,
    /// Some(()) once exclusive has taken this event.
    claim: Option<()>,
}

impl AscentState {
    pub fn new() -> Self {
        Self {
            invalidation_depth: 0,
            ascent_hops: 0,
            claim: None,
        }
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

    /// One-way trap door. Some(()) if open (now taken). None if already taken.
    fn claim(&mut self) -> Option<()> {
        if self.claim.is_some() {
            None
        } else {
            self.claim = Some(());
            Some(())
        }
    }

    /// Kill only. invalidation_depth = invalidation_depth.max(d).
    fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Framework only: one hop of parent recovery finished; about to run parent posts.
    fn bump_ascent_hop(&mut self) {
        self.ascent_hops = self.ascent_hops.saturating_add(1);
    }
}

/// User posts receive this — not bare AscentState.
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

/// User exclusive bodies receive this — not bare AscentState.
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

    /// Trap door. Some(()) if this call took it.
    pub fn claim(&mut self) -> Option<()> {
        self.state.claim()
    }

    /// Kill coverage. d = path hops climbed for the kill (into_parent count).
    pub fn invalidate(&mut self, d: u32) {
        self.state.invalidate(d);
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

Unchanged. Does not know `AscentState`. Does not bump any invalidation counter.

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

    /// Recover parent only. Does not touch AscentState.
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

## `into_parent_ascent` (bind)

Framework hop. **Does not** call `invalidate` / change `invalidation_depth`.

```rust
pub fn into_parent_ascent<Node, Parent, E>(
    path: laserbeam::PathMut<Node, Parent>,
    sink: &mut Vec<E>,
    state: &mut AscentState,
    run_posts: impl FnOnce(Parent, PostCtx<'_>) -> (Parent, Vec<E>),
) -> Parent {
    let parent = path.into_parent();
    state.bump_ascent_hop();
    let (parent, post_effs) = run_posts(parent, PostCtx { state });
    sink.extend(post_effs);
    parent
}
```

## Helpers (bind)

```rust
pub fn run_post<P, E>(
    path: P,
    ctx: PostCtx<'_>,
    body: impl FnOnce(Node<P, ()>, PostCtx<'_>) -> (Vec<E>, P),
) -> (P, Vec<E>) {
    body(Node { parent: path, data: () }, ctx)
}

pub fn run_exclusive<P, E>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, ExclusiveCtx<'_>) -> (Vec<E>, P),
) -> (P, Vec<E>) {
    let mut ctx = ExclusiveCtx { state };
    match ctx.claim() {
        None => (path, Vec::new()),
        Some(()) => body(Node { parent: path, data: () }, ctx),
    }
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

## Kill and path hops

Exclusive climbs with path `into_parent` (same laserbeam recovery). **Each kill hop** counts toward `d`. After the climb (or as it goes), exclusive calls `ctx.invalidate(d)`.

```rust
fn inner_handler(
    _ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    mut ctx: ExclusiveCtx<'_>,
) -> (Vec<DemoEffect>, InnerPath) {
    // Kill covers two framework hops above this exclusive (Outer + one more).
    // The path climb for reshape is what *defines* d; invalidate records it.
    // Until reshape carrier exists, exclusive only records coverage:
    ctx.invalidate(2);
    (vec![], node.parent)
}
```

When reshape carrier exists: each `path.into_parent()` on the kill climb increments a hop counter, then `invalidate(hops)` (or `invalidate` once at end with total hops). Framework `into_parent_ascent` still only `bump_ascent_hop`, never `invalidate`.

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
DESCENT: schedule opts

if child:
  child_path = PathMut::from_fn(path, proj_mut, proj_ref)
  (child_path, state) = Child::dispatch(child_path, event, effs)
  path = into_parent_ascent(child_path, effs, &mut state, |parent, post_ctx| {
    // posts with PostCtx
  })
  // = into_parent(); bump_ascent_hop(); run_posts;  NOT invalidate

if leaf:
  state = AscentState::new()

if exclusive scheduled:
  run_exclusive(path, &mut state, |node, excl_ctx| handler(ev, node, excl_ctx))

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
    // d = kill path hop count (two into_parents of coverage)
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
            let (path, out_effs) = ::bind::run_exclusive(path, &mut state, |node, ctx| {
                inner_handler(ev, node, ctx)
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
            move |parent, ctx| {
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
            },
        );

        if let ::core::option::Option::Some(ev) = opt_2 {
            let (p, e) = ::bind::run_exclusive(path, &mut state, |node, ctx| {
                outer_handler(ev, node, ctx)
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
DESCENT Outer: opt_0 Some(id), opt_1 true, opt_2 Some(ev)
DESCENT Inner: opt_0 Some(ev)

ASCENT Inner
  state = new()                          // inv=0, hops=0, claim=None
  run_exclusive: claim → Some(())
  inner_handler: invalidate(2)           // inv=2
  mutation at leaf: hops 0 < 2 → MaybeDropped (unused; no posts at leaf)
  return (InnerPath, state)

ASCENT Outer into_parent_ascent
  parent = path.into_parent()            // path only; inv still 2
  bump_ascent_hop()                      // hops = 1
  posts with PostCtx:
    mutation: 1 < 2 → MaybeDropped
    after_child → LogDestroyed(id)
    only_if_intact → skip rearm
  // inv still 2 — framework hop did not invalidate/step invalidation_depth

ASCENT Outer exclusive
  claim → None
  outer_handler not run

return hops=1, inv=2, claimed
// next hop to Root: bump hops=2; posts see 2 < 2 → Intact
```

## Walk: KeyB

```text
DESCENT: AnyKey posts scheduled; no KeyA exclusive

ASCENT Inner: state = new(); no exclusive
ASCENT Outer into_parent_ascent:
  bump hops=1
  inv=0 → mutation Intact (1 < 0 is false)
  after_child Intact; rearm runs
  no exclusive
```

## Ordered changes

### P0 — Effect batch + Break

### P1 — optional sink thread

### P2 — from_fn framework-only

### P3 — AscentState + PostCtx + ExclusiveCtx + into_parent_ascent

Full types above. No `step_up` on invalidation_depth. Framework bumps `ascent_hops` only.

### P4 — bind through run_exclusive + ExclusiveCtx

### F1 — post + PostCtx

### F2 — invalidate(d) from exclusive (d from kill path hops)

### F3 — pre_post

### F4 — only_if_intact + rearm

### F5 — reshape carrier: path into_parent on kill counts hops into invalidate(d)

## Rules

1. Descent schedules; set final.
2. Ascent runs every scheduled post.
3. User code gets `PostCtx` / `ExclusiveCtx`, not bare `AscentState`.
4. laserbeam `into_parent` recovers parent only — no depth.
5. Kill / path climb sets `invalidation_depth` via `invalidate`.
6. Framework hop sets `ascent_hops` via `bump_ascent_hop` before parent posts.
7. `mutation()` = MaybeDropped iff `ascent_hops < invalidation_depth`.
8. `claim()` is a one-way trap door (`Option<()>`).
9. Generate: schedule + helpers. Expand above is the template.

## Tests

- `invalidate(2)`: after one framework hop, posts MaybeDropped; after two, Intact
- framework hop does not change `invalidation_depth`
- bare `AscentState` methods `claim`/`invalidate`/`bump_ascent_hop` are not callable from user modules (only via ctx / bind helpers)
- claim trap door; parent exclusive skips
- KeyA / KeyB walks match above
