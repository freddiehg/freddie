# Invalidation: descent schedules, ascent runs posts

Not done. Depends on `path-peel-complete.md` (`Completed` / `Stop` / `Complete` / `Completed::up`).

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

## Model

Every node's dispatch returns `Completed<Self::Path<'a>>`, root included. There is no per-node ascent type and no associated type: the return is the same expression of the node's own path everywhere.

A non-root parent sees three outcomes, from two nested `into_inner` matches:

```rust
match Child::dispatch(child_path, event, effs, claim).into_inner() {
    Stop::Here(child_path) => {
        // child kept focus; this node's path is child_path.into_parent()
    }
    Stop::Up(rest) => match rest.into_inner() {
        Stop::Here(path) => {
            // the leave stopped at this node; child dropped, this node lives
        }
        Stop::Up(above) => {
            // this node dropped too; forward Completed::up(above)
        }
    },
}
```

The `Up` payload of a child's `Completed` is this node's own `Completed`, so the no-inspection form forwards `rest` unchanged; the inspecting form rebuilds its gone-above arm with `Completed::up`. At the root the child's `Up` payload is the bare root path, so the root sees two arms (the demo below).

Posts per arm:

```text
Here(child)            child survived    → posts with the live path; own binds (claim-gated)
Up(Here(this))         child dropped     → posts with the live path (this node's)
Up(Up(above))          this node dropped → posts from pre-descent snaps only
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
        match self.slot.replace(()) {
            Some(()) => None,
            None => Some(()),
        }
    }
}
```

Every bind handler runs behind `try_take`, and its `Completed` is returned directly.

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

## The demo: `A → B`, everything the user writes

`A` is the root and holds the layer `B`. `B` arms a return-home timer; every key while `B` is up pushes the deadline out; leaving `B` must cancel the timer, because the OS timer outlives the state that armed it and `Drop` cannot emit the cancel.

```rust
type APath<'a> = &'a mut A;
type BPath<'a> = PathMut<B, APath<'a>>;

#[derive(Clone, Copy)]
struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        Self(1)
    }
}

struct TimerGuard {
    id: TimerId,
}

// #[bind], #[node], #[binds], #[resolve_into] are today's derive surface.
// #[post] (alive arms, live path) and #[pre_post] (dropped arms, snap only)
// are new in ordered change 5.
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(KeyEsc => flash)]                                               // opt_2
#[post(AnyKey => rearm)]                                               // opt_1
#[pre_post(AnyKey, pre = snap_return_home, post = cancel_return_home)] // opt_0
struct A {
    #[resolve_into]
    b: B,
}

#[derive(Bind)]
#[node(parent = APath)]
#[binds(M)]
#[bind(KeyH => go_home)]
struct B {
    return_home: TimerGuard,
}

enum DemoEffect {
    ScheduleTimer(TimerId),
    CancelTimer(TimerId),
    FlashOverlay,
}

/// The Bindings marker: `M: Bindings<Output = Vec<DemoEffect>>`.
struct M;
```

The handlers and posts, all user-written:

```rust
/// B's bind: go home. The layer is replaced, so B and the guard it holds drop.
fn go_home<'a>(
    _ev: &KeyEvent,
    path: BPath<'a>,
) -> (Vec<DemoEffect>, Completed<BPath<'a>>) {
    (vec![], path.into_parent().complete()) // Up(a)
}

/// A's bind: fires only when nothing deeper claimed the key.
fn flash<'a>(
    _ev: &KeyEvent,
    path: APath<'a>,
) -> (Vec<DemoEffect>, Completed<APath<'a>>) {
    (vec![DemoEffect::FlashOverlay], path.complete())
}

/// A's post while B is alive: any key pushes B's return-home deadline out.
fn rearm(a: &mut A) -> Vec<DemoEffect> {
    let fresh = TimerId::fresh();
    let old = core::mem::replace(&mut a.b.return_home, TimerGuard { id: fresh });
    vec![DemoEffect::CancelTimer(old.id), DemoEffect::ScheduleTimer(fresh)]
}

/// A's pre: runs before descending into B, while B still exists.
fn snap_return_home(_ev: &KeyEvent, a: &A) -> TimerId {
    a.b.return_home.id
}

/// A's post when B dropped: the snap is all that is left of the timer.
fn cancel_return_home(id: TimerId) -> Vec<DemoEffect> {
    vec![DemoEffect::CancelTimer(id)]
}
```

The user never writes an arm match and never sees `Stop`: which body runs on which arm is declared (`post` = alive, `pre_post` = dropped) and the macro emits the arms.

## Generated: B

```rust
impl Dispatch<M> for B {
    fn dispatch<'a, 'c>(
        path: BPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<BPath<'a>>
    where
        Self: 'a,
    {
        let opt_0: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyH;
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
                let (e, completed) = go_home(ev, path);
                effs.extend(e);
                return completed;
            }
        }

        ::laserbeam::Complete::complete(path) // Here(b)
    }
}
```

## Generated: A

```rust
impl Dispatch<M> for A
where
    B: Dispatch<M>,
{
    fn dispatch<'a, 'c>(
        path: &'a mut A,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<&'a mut A>
    where
        Self: 'a,
    {
        // Pre-descent: snaps and the schedule, final before B runs.
        let opt_0: Option<TimerId> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(snap_return_home(ev, path))
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
            let trigger = KeyEsc;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        let b_path = laserbeam::PathMut::from_fn(path, |a| &mut a.b, |a| &a.b);

        match B::dispatch(b_path, event, effs, claim).into_inner() {
            Stop::Here(b_path) => {
                let path = b_path.into_parent();
                if opt_1 {
                    effs.extend(rearm(path));
                }
                if let Some(ev) = opt_2 {
                    if claim.try_take().is_some() {
                        let (e, completed) = flash(ev, path);
                        effs.extend(e);
                        return completed;
                    }
                }
                ::laserbeam::Complete::complete(path)
            }
            Stop::Up(path) => {
                // B dropped; the leave stopped here at the root.
                if let Some(id) = opt_0 {
                    effs.extend(cancel_return_home(id));
                }
                ::laserbeam::Complete::complete(path)
            }
        }
    }
}
```

A deeper tree gets the three-arm match from Model: alive posts in both live arms, snap-only posts in the gone-above arm, `Completed::up(above)` forwarding.

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

### KeyH: B goes home

```text
B:  claim take; go_home returns path.into_parent().complete() → Up(a)
A:  Up(path); cancel_return_home(snapped id) → [CancelTimer]
    return path.complete()
```

The timer armed by the dead layer is cancelled from the snap, which is the only place its id survived.

### Any other key: B stays

```text
B:  fallthrough → Here(b)
A:  Here(b) → path = b.into_parent(); rearm(path)
    → [CancelTimer(old), ScheduleTimer(fresh)]
    KeyEsc additionally: flash claims → [FlashOverlay], returns Here
```

`rearm` runs whether or not anything claimed: posts are scheduled by their trigger, not by the claim.

## Rules

1. No stubs.
2. Arms `Here` / `Up`; three outcomes at a non-root parent via two nested `into_inner` matches; the no-inspection form forwards `rest` unchanged.
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent; the schedule is final; ascent runs every scheduled post.
5. Every bind handler runs behind the claim and returns `(effects, Completed<Self::Path>)`; staying put is `path.complete()`. Posts return effects only.
6. The user writes triggers, handlers, pres, and posts; the macro writes every arm match. `Stop` never appears in user code.
7. Claim separate; one exclusive handler per dispatch.
8. path-peel-complete ships first, including `Completed::up`.

## Tests

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- three-arm coverage on a three-level tree (kept / stopped-at-mid / gone-above)
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts fire without a claim (rearm on an unbound key)

## Ordered changes

Skeletal; flesh out after the design above is agreed. Prefactors first, each independently shippable.

### 1 — bind: `effs` sink + `Claim` on `Dispatch`/`Descend`; drop `ControlFlow` (behavior-preserving)

### 2 — bind: dispatch returns `Completed<Self::Path>` / `Completed<Self::Parent>`; generated matches forward, fallthrough `complete()` (no posts yet)

### 3 — bind_macro: snap trigger opts before descent (the schedule becomes final pre-descent)

### 4 — bind handlers return `(effects, Completed<Path>)`, claim-gated; staying put is `path.complete()`; kill = extra `into_parent`

### 5 — `#[post]` (alive arms, live path) and `#[pre_post]` (dropped arms, snap only)

### 6 — Here-only path-mutation posts and any remaining arm refinements
