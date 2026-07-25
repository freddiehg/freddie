# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

Leave peels (`after_first_peel` / `into_parent` / `complete` → nested **Here/Up** doll) are **`path-peel-complete.md`**. Ship that prefactor first. Pack/Path/PeelPack live only there.

## Model

Spine `A → B → C`. `C::dispatch` always returns `C`’s ascent type (`AscentOf` / node `Ascent` around the doll).

```text
after_first_peel(c_path).complete()                 // Here(b)
after_first_peel(c_path).into_parent().complete()   // Up(Here(a)) or Up(a)
```

Public match arms are `Here` / `Up` only (not `Ok`/`Err`, not `ControlFlow`).

## Types (dispatch layer)

```rust
// Pack / Path / PeelPack / Doll / after_first_peel / complete / AscentOf:
// see path-peel-complete.md
//
// Doll<H, U> { Here(H), Up(U) }  — public arms
// AscentOf<P> = <P as AscentOut>::Out

// Optional opaque wrapper if we hide the nest further:
pub struct Ascent<P: AscentOut> {
    doll: P::Out,
}

impl<P: AscentOut> Ascent<P> {
    pub fn new(doll: P::Out) -> Self {
        Self { doll }
    }

    pub fn into_inner(self) -> P::Out {
        self.doll
    }
}

// Match child leave (same as matching Doll):
// match child_ascent { Doll::Here(path) | Doll::Up(rest) => ... }
// If wrapped: match ascent.into_inner() { ... }

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

See path-peel-complete. In Here/Up:

```text
after_first_peel(c).complete()                              Here(b)
after_first_peel(c).into_parent().complete()                Up(Here(root))  // typical nest
after_first_peel(inner).into_parent().complete()            Up(root)        // bare rest
```

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

`Place::Path<'a>` stays the laserbeam path (`PathMut<…>` or `&mut Root`). Bind `Path<_, pack>` is only for leave/kill peels after `after_first_peel`.

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
    let ascent = after_first_peel(path).into_parent().complete();
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

        after_first_peel(path).complete() // Here(outer)
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

Why Outer does not use `after_first_peel`: `Outer::Ascent` is bare `RootPath`, not a `Doll` layer. One laserbeam `into_parent` yields root. Pack/`complete` apply when the node’s leave doll is a `Doll` (still has a parent layer).

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
  after_first_peel(inner).into_parent().complete()
  → Up(root)
Outer match Up(root):
  after_child_dropped(snap)
  return root
Root: ()
```

### KeyB, no kill

```text
Inner: after_first_peel.complete() → Here(outer)
Outer match Here(outer):
  after_child_ok, rearm, maybe exclusive
  outer.into_parent() → root
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

### P1 — path-peel-complete (`Doll` Here/Up, Pack, Path, tests)

Unit tests: one/two/three peels; bare-root second peel `Up(root)`.

### P2 — Dispatch::Ascent + Claim; Inner/Outer/Root expands as above

### P3 — bind_macro emits schedule opts, unpack, after_first_peel, claim exclusive

### F1 — `#[post]` on Here and Up

### F2 — kill = extra `into_parent` before `complete` in exclusive

### F3 — `#[pre_post]` pre-snap for Up

### F4 — Here-only path mutation posts

## Rules

1. No stubs.
2. Public doll arms are `Here` / `Up` only.
3. Leave/kill: `after_first_peel` / `into_parent` / `complete`; `Out = AscentOf<OriginPath>`.
4. Node whose ascent is bare path (Outer → Root): laserbeam `into_parent` only.
5. Claim separate.
6. path-peel-complete ships first.

## Tests

- path-peel-complete nest tests (`Here` / `Up(Here)` / `Up`)
- KeyA / KeyB walks on Outer/Inner expand
- claim trap door
