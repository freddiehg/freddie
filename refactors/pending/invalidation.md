# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

Leave peels (`after_first_peel` / `into_parent` / `complete` → nested `Result`) are documented in **`path-peel-complete.md`**. That is a separate prefactor; ship it first. Pack/Path/PeelPack types live only there.

## Model

Spine `A → B → C`. `C::dispatch` always returns `C`’s ascent type (opaque `Ascent` around the doll from path-peel-complete).

```text
after_first_peel(c_path).complete()                 // Ok(b)
after_first_peel(c_path).into_parent().complete()   // Err(Ok(a)) or Err(a)
```

## Types (dispatch layer)

```rust
// Pack / Path / PeelPack / after_first_peel / complete: see path-peel-complete.md

pub struct Ascent<D> {
    doll: D,
}

impl<D> Ascent<D> {
    pub fn new(doll: D) -> Self {
        Self { doll }
    }

    pub fn into_inner(self) -> D {
        self.doll
    }
}

pub enum Step<Here, Up> {
    Here(Here),
    Up(Up),
}

pub fn unpack<H, U>(doll: Result<H, U>) -> Step<H, U> {
    match doll {
        Ok(h) => Step::Here(h),
        Err(u) => Step::Up(u),
    }
}

pub fn unpack_ascent<H, U>(ascent: Ascent<Result<H, U>>) -> Step<H, U> {
    unpack(ascent.into_inner())
}

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

```rust
// BPath = PathMut<B, APath>, APath = PathMut<A, Z>
// CAscent = Result<BPath, Result<APath, Result<Z, ()>>>

let at_b = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path);
let _: CAscent = at_b.complete(); // Ok(b)

let at_a = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path).into_parent();
let _: CAscent = at_a.complete(); // Err(Ok(a))

let at_z = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path)
    .into_parent()
    .into_parent();
let _: CAscent = at_z.complete(); // Err(Err(Ok(z)))
```

Bare root rest (demo Inner → Outer → Root):

```rust
// InnerAscent = Result<OuterPath, RootPath>
// OuterPath = PathMut<Outer, RootPath>

let at_outer = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(inner_path);
let _: Result<OuterPath<'a>, RootPath<'a>> = at_outer.complete(); // Ok(outer)

let at_root = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(inner_path).into_parent();
// PeelPack: AsHere<RootPath> → AsUp<OuterPath, AsTerminal>
let _: Result<OuterPath<'a>, RootPath<'a>> = at_root.complete(); // Err(root)
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

// Inner::Ascent<'a> = Ascent<Result<OuterPath<'a>, RootPath<'a>>>
// Outer::Ascent<'a> = Ascent<RootPath<'a>>

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
) -> (Vec<DemoEffect>, Ascent<Result<OuterPath<'a>, RootPath<'a>>>) {
    let ascent = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(path)
        .into_parent()
        .complete();
    (vec![], Ascent::new(ascent))
}
```

## Generated: Inner

```rust
impl Dispatch<M> for Inner {
    type Ascent<'a> = Ascent<Result<OuterPath<'a>, RootPath<'a>>>
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

        let doll = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(path).complete();
        Ascent::new(doll)
    }
}
```

## Generated: Outer

```rust
impl Dispatch<M> for Outer
where
    Inner: Dispatch<M>,
{
    type Ascent<'a> = Ascent<RootPath<'a>>
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

        match unpack_ascent(Inner::dispatch(inner_path, event, effs, claim)) {
            Step::Here(mut outer_path) => {
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
                // Outer::Ascent has no Result layer: laserbeam peel is the whole leave.
                Ascent::new(outer_path.into_parent())
            }
            Step::Up(root_path) => {
                if let Some(id) = opt_0 {
                    effs.extend(after_child_dropped(id));
                }
                Ascent::new(root_path)
            }
        }
    }
}
```

Why Outer does not use `after_first_peel`: `Outer::Ascent` is `Ascent<RootPath>`, not `Ascent<Result<_, _>>`. One laserbeam `into_parent` yields `RootPath`. Pack/`complete` apply only when the node’s ascent doll is a `Result` (child of something that still has a parent layer in the doll).

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

        let ascent = Outer::dispatch(outer_path, event, effs, claim);
        // Outer::Ascent = Ascent<RootPath>; recover root if needed, then done.
        let _root_path = ascent.into_inner();
        // _root_path: RootPath = &mut Root — same borrow ended when path dropped;
        // free dispatch only needs effects + claim.
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
  → Err(root) inside Ascent
Outer unpack Up(root):
  after_child_dropped(snap)
  return Ascent(root)
Root:
  into_inner, ()
```

### KeyB, no kill

```text
Inner: after_first_peel.complete() → Ok(outer)
Outer unpack Here(outer):
  after_child_ok, rearm, maybe exclusive
  outer.into_parent() → Ascent(root)
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

### P1 — Pack / Path / PeelPack / AsTerminal / after_first_peel / Ascent / unpack

Unit tests: one/two/three peels; bare-root second peel `Err(root)`.

### P2 — Dispatch::Ascent + Claim; Inner/Outer/Root expands as above

### P3 — bind_macro emits schedule opts, unpack, after_first_peel, claim exclusive

### F1 — `#[post]` Here and Up

### F2 — kill = extra `into_parent` before `complete` in exclusive

### F3 — `#[pre_post]` pre-snap for Up

### F4 — Here-only path mutation posts

## Rules

1. No stubs.
2. Leave/kill: `after_first_peel` / `into_parent` / `complete`; `Out = Pack::Out`.
3. `AsHere<Parent>` peels to terminal with `AsTerminal` (bare root rest).
4. `AsHere<Result<Parent, E>>` peels with nested `AsHere<E>`.
5. Node whose ascent is bare path (Outer → Root): laserbeam `into_parent` only.
6. Private doll in `Ascent`; unpack via `Step`.
7. Claim separate.

## Tests

- `after_first_peel` + `complete` → `Ok`
- `after_first_peel` + `into_parent` + `complete` with `Rest = RootPath` → `Err(root)`
- nested `Rest = Result<…>` three peels
- KeyA / KeyB walks on Outer/Inner expand
- claim trap door
