# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

Leave peels on **`Place::Path`** (`PathMut` vs root `&mut Self` as the bind macro already distinguishes) are **`path-peel-complete.md`**. Ship that prefactor first.

## Model

Each non-root node returns `Ascent(self) = Doll<Self::Path, Ascent(parent)>` (root path bare). Child returns `Ascent(child)`; parent matches `Here`/`Up` and returns `Ascent(self)`. App only ever sees `LayerAscent`, not `NavAscent`.

```text
leave_at_nav(nav).complete()                              // Here(nav)
leave_at_nav(nav).into_parent().complete()                // Up(Here(layer))
leave_at_nav(nav).into_parent().into_parent().complete()  // Up(Up(app))
```

Public arms: `Here` / `Up` only. LeavePath is `LeavePath<Focus, Origin>` — see path-peel-complete.

## Types (dispatch layer)

```rust
// Doll, LeavePath<Focus, Origin>, ascent aliases, leave_at_*, complete:
// see path-peel-complete.md
//
// Ascent(root) = &mut Root
// Ascent(node) = Doll<Node::Path, Ascent(parent)>
// e.g. LayerAscent, NavAscent = Doll<NavPath, LayerAscent>
//
// match child_ascent { Doll::Here(path) | Doll::Up(rest) => ... }
// Up(rest) already has type Ascent(this); return rest upward unchanged

// ---------------------------------------------------------------------------
// Claim
// ---------------------------------------------------------------------------

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

laserbeam `PathMut::into_parent(self) -> Parent` unchanged.

### Worked peels

See path-peel-complete (`leave_at_nav` / `leave_at_layer` on real Place paths).

## Dispatch

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}

pub trait Dispatch<M: Bindings>: Place {
    type Ascent<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a;
}
```

`Place::Path<'a>` is unchanged. `LeavePath<Focus, Origin>` for leave/kill; `into_parent` generic on `PathMut`.

## DX types

```rust
// RootPath<'a>  = &'a mut Root
// OuterPath<'a> = laserbeam::PathMut<Outer, RootPath<'a>>
// InnerPath<'a> = laserbeam::PathMut<Inner, OuterPath<'a>>

// Inner::Ascent<'a> = AscentOf<InnerPath<'a>>
//   = Doll<OuterPath<'a>, RootPath<'a>>   (or Ascent wrapper around that)
// Outer::Ascent<'a> = AscentOf<OuterPath<'a>>
//   = RootPath<'a>   (boundary; no Doll layer)

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

fn outer_handler(_ev: &KeyEvent, _outer: &mut OuterPath<'_>) -> Vec<DemoEffect> {
    vec![DemoEffect::SetLayerHome]
}

fn inner_handler<'a>(
    _ev: &KeyEvent,
    path: InnerPath<'a>,
) -> (Vec<DemoEffect>, AscentOf<InnerPath<'a>>) {
    let ascent = leave_at_inner(path).into_parent().complete()  // see path-peel-complete naming;
    (vec![], ascent)
}
```

## Generated: Inner

```rust
impl Dispatch<M> for Inner {
    type Ascent<'a> = AscentOf<InnerPath<'a>>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: InnerPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
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
            if let Some(()) = claim.try_take() {
                let (e, ascent) = inner_handler(ev, path);
                effs.extend(e);
                return ascent;
            }
        }

        /* leave complete at this path — Here */
    }
}
```

## Generated: Outer

```rust
impl Dispatch<M> for Outer
where
    Inner: Dispatch<M>,
{
    type Ascent<'a> = AscentOf<OuterPath<'a>> // = RootPath<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: OuterPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
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

        match Inner::dispatch(inner_path, event, effs, claim) {
            Doll::Here(mut outer_path) => {
                if let Some(id) = opt_0 {
                    effs.extend(after_child_ok(id, &mut outer_path));
                }
                if opt_1 {
                    effs.extend(rearm(&mut outer_path));
                }
                if let Some(ev) = opt_2 {
                    let e = claim.with_exclusive(&mut outer_path, |p| outer_handler(ev, p));
                    effs.extend(e);
                }
                // Outer::Ascent is bare RootPath: laserbeam peel only.
                outer_path.into_parent()
            }
            Doll::Up(root_path) => {
                if let Some(id) = opt_0 {
                    effs.extend(after_child_dropped(id));
                }
                root_path
            }
        }
    }
}
```

Outer/Layer with parent root: `LayerAscent = Doll<LayerPath, AppPath>`. App matches that only — never NavAscent.

## Generated: Root (struct with one `#[resolve_into]` child)

```rust
// Place::Path<'a> = &'a mut Root
// Root::Ascent<'a> = ()  // nothing above root

impl Dispatch<M> for Root
where
    Outer: Dispatch<M>,
{
    type Ascent<'a> = ()
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: &'a mut Root,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let outer_path = laserbeam::PathMut::from_fn(
            path,
            |r: &mut Root| &mut r.outer,
            |r: &Root| &r.outer,
        );

        let _root_path = Outer::dispatch(outer_path, event, effs, claim);
        // Outer::Ascent = RootPath; free dispatch only needs effects + claim.
        ()
    }
}
```

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
    let _ascent = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
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
Inner exclusive:
  claim take
  leave from inner → Up(…) : LayerAscent payload toward outer
Outer match:
  after_child_dropped(snap)
  return root
Root: ()
```

### KeyB, no kill

```text
Child Here(path) / Up(ascent_of_this_node); this node returns Ascent(self) to parent.
App receives only LayerAscent.
```

## Ordered changes

### P0 — Sink; drop ControlFlow

Before (`bind`):

```rust
fn dispatch(path, event) -> ControlFlow<M::Output, Path>;
// child: let child = Child::dispatch(...)?;  // Break propagates
```

After:

```rust
fn dispatch(path, event, effs: &mut M::Output, claim: &mut Claim) -> Self::Ascent;
// child always returns ascent; parent unpacks; no ?
```

Free `dispatch` builds `effs` + `claim`, returns `Option<M::Output>`.

### P1 — path-peel-complete (`Doll`, `LeavePath<Focus, Origin>`, recursive ascent aliases, tests)

trybuild over-peel; multi-depth unify to `NavAscent`; mut through recovered paths.

### P2 — Dispatch::Ascent + Claim; Inner/Outer/Root expands as above

### P3 — bind_macro emits schedule opts, leave_at/complete, claim exclusive

### F1 — `#[post]` on Here and Up

### F2 — kill = extra `into_parent` before `complete` in exclusive

### F3 — `#[pre_post]` pre-snap for Up

### F4 — Here-only path mutation posts

## Rules

1. No stubs.
2. Public doll arms are `Here` / `Up` only.
3. Leave/kill: `LeavePath<Focus, Origin>`; `Ascent(node) = Doll<path, Ascent(parent)>`.
4. Each node receives `Ascent(child)`, returns `Ascent(self)`. App sees only child-of-root ascent.
5. Posts on `Up` deferred (invalidation policy). Claim separate.
6. path-peel-complete ships first.

## Tests

- path-peel-complete tests (smoke, over-peel fail, unify depths, usable paths)
- KeyA / KeyB walks; App only matches LayerAscent
- claim trap door
