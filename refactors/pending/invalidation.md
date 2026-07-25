# Invalidation: descent schedules, ascent runs posts

Not done. Depends on `path-peel-complete.md` (`Completed` / `Stop` / `Complete` / `Completed::up`).

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

## Model

Every node's dispatch returns `Completed<Self::Path<'a>>`, root included. There is no per-node ascent type and no associated type: the return is the same expression of the node's own path everywhere.

A parent sees three outcomes, from two nested `into_inner` matches:

```rust
match Inner::dispatch(inner_path, event, effs, claim).into_inner() {
    Stop::Here(inner_path) => {
        // child kept focus; this node's path is inner_path.into_parent()
    }
    Stop::Up(rest) => match rest.into_inner() {
        Stop::Here(outer_path) => {
            // the leave stopped at this node; child dropped, this node lives
        }
        Stop::Up(above) => {
            // this node dropped too; forward Completed::up(above)
        }
    },
}
```

The `Up` payload of a child's `Completed` is this node's own `Completed`, so the no-inspection form forwards `rest` unchanged; the inspecting form rebuilds its gone-above arm with `Completed::up`.

Posts per arm:

```text
Here(child)            child survived   → child-ok posts; own binds (claim-gated)
Up(Here(this))         child dropped    → child-dropped posts; posts that need this path
Up(Up(above))          this node dropped → child-dropped posts from pre-descent snaps only
```

## Claim

```rust
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
        path: &mut P,
        body: impl FnOnce(&mut P) -> Vec<E>,
    ) -> Vec<E> {
        match self.try_take() {
            None => Vec::new(),
            Some(()) => body(path),
        }
    }
}
```

`with_exclusive` is for in-place handlers (effects only). A leaving handler takes the claim with `try_take` and its `Completed` is returned directly (see Generated: Inner).

## Dispatch

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> ::laserbeam::Completed<Self::Path<'a>>
    where
        Self: 'a,
        Self::Path<'a>: ::laserbeam::HasStop;
}
```

The root's `Path` is `&mut Root`, whose `Completed` wraps the bare path, so the same signature serves the root; the free dispatch drops it. bind now names laserbeam traits (`HasStop` in the bound, `Complete` in expansions), not only its types; `Place`'s doc comment updates accordingly.

Derived levels live at their parent place, so their leave is the parent's:

```rust
pub trait Descend<M: Bindings>: HasParent + Sized
where
    Self::Parent: ::laserbeam::HasStop,
{
    fn dispatch<'c>(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> ::laserbeam::Completed<Self::Parent>;
}
```

`Here(parent_path)` means the derived level kept focus at the place it lives at. A root-parented derived level works because `Completed<&mut Root>` exists. `EventHandler` / `DerivedHandler` (the check) are untouched.

## Handlers

Two shapes, distinguished by whether the handler leaves:

```rust
// in place: effects only; runs under with_exclusive or as a post
fn outer_handler(_ev: &KeyEvent, _outer: &mut OuterPath<'_>) -> Vec<DemoEffect> {
    vec![DemoEffect::SetLayerHome]
}

// leaving: effects + where dispatch is afterwards
fn inner_handler<'a>(
    _ev: &KeyEvent,
    path: InnerPath<'a>,
) -> (Vec<DemoEffect>, Completed<InnerPath<'a>>) {
    (vec![], path.into_parent().complete()) // Up(Here(outer))
}
```

Kill = more `into_parent` calls before `complete`. Handwritten handlers `use laserbeam::Complete;`.

## DX types

```rust
// RootPath<'a>  = &'a mut Root
// OuterPath<'a> = laserbeam::PathMut<Outer, RootPath<'a>>
// InnerPath<'a> = laserbeam::PathMut<Inner, OuterPath<'a>>
//
// Inner::dispatch returns Completed<InnerPath<'a>>
// Outer::dispatch returns Completed<OuterPath<'a>>
// Root::dispatch  returns Completed<RootPath<'a>>   (bare &mut Root inside)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChildId(u64);

struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        Self(1)
    }
}

struct TimerGuard {
    id: TimerId,
}

struct AndReturnHome {
    guard: TimerGuard,
}

struct Outer {
    inner: Inner,
    return_home: AndReturnHome,
}

struct Inner {
    id: ChildId,
}

enum DemoEffect {
    LogDestroyed(ChildId),
    ScheduleTimer(TimerId),
    SetLayerHome,
}

struct M;

// In real code M: Bindings with Output = Vec<DemoEffect>

fn log_destroyed(id: ChildId) -> DemoEffect {
    DemoEffect::LogDestroyed(id)
}

fn arm_return_home() -> (TimerGuard, DemoEffect) {
    let id = TimerId::fresh();
    (TimerGuard { id }, DemoEffect::ScheduleTimer(id))
}

fn snap_child_id(_ev: &KeyEvent, outer: &OuterPath<'_>) -> ChildId {
    outer.get().inner.id
}

fn after_child_ok(id: ChildId, outer: &mut OuterPath<'_>) -> Vec<DemoEffect> {
    debug_assert_eq!(outer.get().inner.id, id);
    vec![]
}

fn after_child_dropped(id: ChildId) -> Vec<DemoEffect> {
    vec![log_destroyed(id)]
}

fn rearm(outer: &mut OuterPath<'_>) -> Vec<DemoEffect> {
    let (guard, schedule) = arm_return_home();
    outer.get_mut().return_home.guard = guard;
    vec![schedule]
}
```

## Generated: Inner

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a, 'c>(
        path: InnerPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<InnerPath<'a>>
    where
        Self: 'a,
    {
        let opt_0: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyA;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(ev) = opt_0 {
            if claim.try_take().is_some() {
                let (e, completed) = inner_handler(ev, path);
                effs.extend(e);
                return completed;
            }
        }

        ::laserbeam::Complete::complete(path) // Here(inner)
    }
}
```

## Generated: Outer

```rust
impl Dispatch<M> for Outer
where
    Inner: Dispatch<M>,
{
    fn dispatch<'a, 'c>(
        path: OuterPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<OuterPath<'a>>
    where
        Self: 'a,
    {
        // Pre-descent snaps: the schedule is final before the child runs.
        let opt_0: Option<ChildId> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(snap_child_id(ev, &path))
            } else {
                None
            }
        } else {
            None
        };

        let opt_1: bool = if let Ok(ev) = TryFrom::try_from(event) {
            EventTrigger::is_matching(&AnyKey, ev)
        } else {
            false
        };

        let opt_2: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyA;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        let inner_path = laserbeam::PathMut::from_fn(
            path,
            |p: &mut OuterPath<'a>| &mut p.get_mut().inner,
            |p: &OuterPath<'a>| &p.get().inner,
        );

        match Inner::dispatch(inner_path, event, effs, claim).into_inner() {
            Stop::Here(inner_path) => {
                let mut path = inner_path.into_parent();
                if let Some(id) = opt_0 {
                    effs.extend(after_child_ok(id, &mut path));
                }
                if opt_1 {
                    effs.extend(rearm(&mut path));
                }
                if let Some(ev) = opt_2 {
                    let e = claim.with_exclusive(&mut path, |p| outer_handler(ev, p));
                    effs.extend(e);
                }
                ::laserbeam::Complete::complete(path) // Here(outer)
            }
            Stop::Up(rest) => match rest.into_inner() {
                Stop::Here(path) => {
                    // Leave stopped at Outer: child dropped, Outer lives.
                    if let Some(id) = opt_0 {
                        effs.extend(after_child_dropped(id));
                    }
                    ::laserbeam::Complete::complete(path)
                }
                Stop::Up(above) => {
                    // Outer dropped too: pre-descent snaps only.
                    if let Some(id) = opt_0 {
                        effs.extend(after_child_dropped(id));
                    }
                    Completed::up(above)
                }
            },
        }
    }
}
```

## Generated: Root

```rust
impl Dispatch<M> for Root
where
    Outer: Dispatch<M>,
{
    fn dispatch<'a, 'c>(
        path: &'a mut Root,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<&'a mut Root>
    where
        Self: 'a,
    {
        let outer_path = laserbeam::PathMut::from_fn(
            path,
            |r: &mut Root| &mut r.outer,
            |r: &Root| &r.outer,
        );

        match Outer::dispatch(outer_path, event, effs, claim).into_inner() {
            Stop::Here(outer_path) => {
                let mut path = outer_path.into_parent();
                // root's own binds, claim-gated, as at any node
                ::laserbeam::Complete::complete(path)
            }
            Stop::Up(root_path) => {
                // The leave stopped at the root; it cannot go higher.
                ::laserbeam::Complete::complete(root_path)
            }
        }
    }
}
```

The root's `Up` payload is the bare `&mut Root` (its parent slot in the child's nest), and its own return wraps the bare path. Nothing about the root is a special case in the trait.

## Free dispatch

Repo convention: `Bindings::Output = Vec<E>`. Sink is `&mut M::Output`. Handlers `effs.extend(...)`.

```rust
pub fn dispatch<'a, M, N, E>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<E>>
where
    M: Bindings<Output = Vec<E>>,
    N: Dispatch<M> + 'a,
{
    let mut effs: Vec<E> = Vec::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _completed = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    if claim.is_taken() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

## Walks

### KeyA, inner kills to root

```text
Inner:  claim take; handler returns path.into_parent().into_parent().complete()
        → Up(Up(root))
Outer:  into_inner → Up(rest); rest.into_inner → Up(root_path)
        after_child_dropped(snap); return Completed::up(root_path)
Root:   into_inner → Up(root_path); return root_path.complete()
```

### KeyB, no kill

```text
Inner:  fallthrough → Here(inner)
Outer:  Here(inner) → path = inner.into_parent(); after_child_ok, rearm,
        maybe exclusive; return path.complete()
Root:   Here(outer) → path = outer.into_parent(); return path.complete()
```

## Rules

1. No stubs.
2. Arms `Here` / `Up`; three outcomes at a parent via two nested `into_inner` matches; the no-inspection form forwards `rest` unchanged.
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent; the schedule is final; ascent runs every scheduled post.
5. In-place handlers return effects and run under `with_exclusive`; leaving handlers take the claim and return `(effects, Completed<Path>)`.
6. Claim separate; one exclusive handler per dispatch.
7. path-peel-complete ships first, including `Completed::up`.

## Tests

- KeyA / KeyB walks on the Inner/Outer/Root expansion
- three-arm coverage at Outer (kept / stopped-here / gone-above)
- claim trap door
- root binds fire in the Here arm, claim-gated

## Ordered changes

Skeletal; flesh out after the design above is agreed. Prefactors first, each independently shippable.

### 1 — bind: `effs` sink + `Claim` on `Dispatch`/`Descend`; drop `ControlFlow` (behavior-preserving)

### 2 — bind: dispatch returns `Completed<Self::Path>` / `Completed<Self::Parent>`; generated matches forward, fallthrough `complete()` (no posts yet)

### 3 — bind_macro: snap trigger opts before descent (the schedule becomes final pre-descent)

### 4 — leaving handlers: `(effects, Completed<Path>)`, claim-gated; kill = extra `into_parent` before `complete`

### 5 — `#[post]` on the three arms (kept / stopped-here / gone-above)

### 6 — `#[pre_post]` pre-snaps; `Here`-only path-mutation posts
