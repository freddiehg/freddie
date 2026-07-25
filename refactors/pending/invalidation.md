# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

No stubs. If a type or function is named, it has a body that typechecks. A stub is how a broken pack stack was being hidden.

## Model

Spine `A → B → C`. `C::dispatch` always returns the same type: `C`’s ascent (private nested `Result` doll inside bind).

Path is generic over the pack that builds that ascent. `complete()` returns `Pack::Out`. `into_parent` peels the focus and rewrites the pack so `Out` is unchanged.

```text
path at C, first peel installs AsHere  →  path at B, Out = Result<B, Rest>
into_parent                            →  path at A, pack becomes AsUp, same Out
complete()                             →  always Out
```

With stop at A after two peels from C (`Out = Result<BPath, Result<APath, …>>`):

```text
complete at B  →  Ok(b)
complete at A  →  Err(Ok(a))     // private nest; app sees opaque Ascent
complete deeper → Err(Err(…))
```

## Types (all real)

```rust
use core::marker::PhantomData;

// --- pack: stop path P → Out ---

pub trait Pack<P> {
    type Out;
    fn pack(self, path: P) -> Self::Out;
}

/// This Result layer stops here: Ok(path).
pub struct AsHere<E>(PhantomData<E>);

impl<P, E> Pack<P> for AsHere<E> {
    type Out = Result<P, E>;
    fn pack(self, path: P) -> Result<P, E> {
        Ok(path)
    }
}

/// This Result layer was skipped: Err(inner.pack(path)).
pub struct AsUp<Q, Inner>(Inner, PhantomData<Q>);

impl<Q, Inner, P> Pack<P> for AsUp<Q, Inner>
where
    Inner: Pack<P>,
{
    type Out = Result<Q, Inner::Out>;
    fn pack(self, path: P) -> Self::Out {
        Err(self.0.pack(path))
    }
}

// --- rewrite pack when focus peels PathMut → Parent; Out unchanged ---

pub trait PeelPack<Node, Parent>: Pack<laserbeam::PathMut<Node, Parent>> + Sized {
    type After: Pack<Parent, Out = Self::Out>;
    fn peel_pack(self) -> Self::After;
}

impl<Node, Parent, E> PeelPack<Node, Parent> for AsHere<Result<Parent, E>> {
    type After = AsUp<laserbeam::PathMut<Node, Parent>, AsHere<E>>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsHere(PhantomData), PhantomData)
    }
}

impl<Node, Parent, Q, Inner> PeelPack<Node, Parent> for AsUp<Q, Inner>
where
    Inner: PeelPack<Node, Parent>,
{
    type After = AsUp<Q, Inner::After>;
    fn peel_pack(self) -> Self::After {
        AsUp(self.0.peel_pack(), PhantomData)
    }
}

// --- Path: focus + pack. complete() → Pack::Out (the Out generic is Pack::Out). ---

pub struct Path<P, Pk> {
    focus: P,
    pack: Pk,
}

impl<P, Pk> Path<P, Pk>
where
    Pk: Pack<P>,
{
    pub fn complete(self) -> Pk::Out {
        self.pack.pack(self.focus)
    }

    pub fn focus(&self) -> &P {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut P {
        &mut self.focus
    }
}

impl<Node, Parent, Pk> Path<laserbeam::PathMut<Node, Parent>, Pk>
where
    Pk: PeelPack<Node, Parent>,
{
    /// Peel laserbeam focus; rewrite pack; Pack::Out unchanged.
    pub fn into_parent(self) -> Path<Parent, Pk::After> {
        Path {
            focus: self.focus.into_parent(),
            pack: self.pack.peel_pack(),
        }
    }
}

/// First peel from the node’s own PathMut: install AsHere for Result<Parent, Rest>.
/// Out = Result<Parent, Rest> = this node’s ascent doll root.
pub fn after_first_peel<Node, Parent, Rest>(
    path: laserbeam::PathMut<Node, Parent>,
) -> Path<Parent, AsHere<Rest>> {
    Path {
        focus: path.into_parent(),
        pack: AsHere(PhantomData),
    }
}

// --- unpack one doll layer for parent match (Result stays inside bind) ---

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

// Opaque newtype if we do not want app code to see Result in signatures:
pub struct Ascent<D> {
    doll: D,
}

impl<D> Ascent<D> {
    pub(crate) fn new(doll: D) -> Self {
        Self { doll }
    }

    pub(crate) fn into_doll(self) -> D {
        self.doll
    }
}

pub fn unpack_ascent<H, U>(ascent: Ascent<Result<H, U>>) -> Step<H, U> {
    unpack(ascent.into_doll())
}

// --- Claim ---

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

laserbeam `PathMut` unchanged: `into_parent(self) -> Parent` only. Bind’s `Path` owns pack + `Out`.

### Worked peels (same types as above)

```rust
// CAscent = Result<BPath, Result<APath, Result<Z, ()>>>
// BPath = PathMut<B, APath>, APath = PathMut<A, Z>

// leave one peel (stop at B):
let at_b = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path);
let out: CAscent = at_b.complete(); // Ok(b)

// leave two peels (stop at A):
let at_a = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path)
    .into_parent();
let out: CAscent = at_a.complete(); // Err(Ok(a))

// leave three peels (stop at Z):
let at_z = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c_path)
    .into_parent()
    .into_parent();
let out: CAscent = at_z.complete(); // Err(Err(Ok(z)))
```

`PeelPack` for `AsHere` requires `Rest = Result<Parent, E>` so the next stop type is the `Ok` of `Rest`. That is exactly the nested doll shape. A bare non-`Result` rest cannot peel further — correct at the root boundary.

### Why `Path<P, Out>` alone was a stub

Writing `Path<P, Out> { complete(self) -> Out }` without a `Pack` type that **is** `Out` papers over the problem: you need a value that implements `P -> Out` and a law for how that value changes on `into_parent`. `Pk: Pack<P, Out = Out>` is that value. `Out` is not free; it is `Pk::Out`.

## Dispatch

```rust
pub trait Dispatch<M: Bindings>: Place {
    /// Private doll wrapped in Ascent, or bare Result inside bind only.
    type Ascent<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: laserbeam::PathMut</* node */, /* parent */>, // see leaf/parent expands
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a;
}
```

For a place whose laserbeam path is `PathMut<Self, ParentFocus>` and whose ascent doll is `Result<ParentFocus, Parent::Ascent>`:

```rust
type Ascent<'a> = Ascent<Result<ParentFocus<'a>, <Parent as Dispatch<M>>::Ascent<'a>>>;
```

Root: no `Result` layer; free `dispatch` owns `&mut Root` and only uses child ascent + effects.

## Parent one level up

```rust
match unpack_ascent(child_ascent) {
    Step::Here(parent_focus) => {
        // parent_focus is the laserbeam parent path recovered from child Ok
        // posts + exclusive using parent_focus
        // leave: after_first_peel from a PathMut that wraps parent_focus, or
        // if parent_focus is already the PathMut at this node, first peel toward grandparent:
        let ascent = after_first_peel(this_node_path).complete(); // or more into_parent for kill
        ascent
    }
    Step::Up(rest) => {
        // posts dropped (pre-snap)
        Ascent::new(rest) // if rest is already parent doll Err payload
    }
}
```

Exact parent expand for Outer/Inner is below with full types.

## Claim / posts

- Here: posts get focus path.
- Up: pre-snap only.
- Claim: separate slot; exclusive on Here when this level owns a path.

## DX + full expand

```rust
// Focus types (aliases)
// RootPath<'a> = &'a mut Root
// OuterPath<'a> = laserbeam::PathMut<Outer, RootPath<'a>>
// InnerPath<'a> = laserbeam::PathMut<Inner, OuterPath<'a>>

// Inner::Ascent<'a> = Ascent<Result<OuterPath<'a>, RootPath<'a>>>
// Outer::Ascent<'a> = Ascent<RootPath<'a>>  // boundary: bare root path inside Ascent

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChildId(u64);

struct AndReturnHome {
    guard: TimerGuard,
}

struct TimerGuard {
    id: TimerId,
}

struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        Self(1)
    }
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

/// Kill to root. Returns Inner::Ascent: Err(root) private.
fn inner_handler<'a>(
    _ev: &KeyEvent,
    path: InnerPath<'a>,
) -> (
    Vec<DemoEffect>,
    Ascent<Result<OuterPath<'a>, RootPath<'a>>>,
) {
    let at_outer = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(path);
    let at_root = at_outer.into_parent();
    (vec![], Ascent::new(at_root.complete()))
}
```

### Generated Inner

```rust
impl Dispatch<M> for Inner {
    type Ascent<'a> = Ascent<Result<OuterPath<'a>, RootPath<'a>>>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: InnerPath<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0: Option<&KeyEvent> =
            if let Ok(ev) = TryFrom::try_from(event) {
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

        // normal leave: one peel, complete → Ok(outer)
        let at_outer = after_first_peel::<Inner, OuterPath<'a>, RootPath<'a>>(path);
        Ascent::new(at_outer.complete())
    }
}
```

### Generated Outer

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
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0: Option<ChildId> =
            if let Ok(ev) = TryFrom::try_from(event) {
                let trigger = AnyKey;
                if EventTrigger::is_matching(&trigger, ev) {
                    Some(snap_child_id(ev, &path))
                } else {
                    None
                }
            } else {
                None
            };

        let opt_1: bool =
            if let Ok(ev) = TryFrom::try_from(event) {
                EventTrigger::is_matching(&AnyKey, ev)
            } else {
                false
            };

        let opt_2: Option<&KeyEvent> =
            if let Ok(ev) = TryFrom::try_from(event) {
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

Outer’s ascent is bare `RootPath` inside `Ascent` (no extra `Result`). Child `Ok` gives `OuterPath`; peel with laserbeam `into_parent` only. Child `Err` already is `RootPath`.

## Free dispatch

```rust
pub fn dispatch<'a, M, N>(
    root: &'a mut N,
    event: &M::Event,
) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + Place<Path<'a> = &'a mut N>,
{
    let mut effs = Vec::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    // root builds child path and calls child / self dispatch as today’s tree does
    let _ascent = <N as Dispatch<M>>::dispatch(root, event, &mut effs, &mut claim);
    if claim.is_taken() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

Root’s concrete expand is the existing enum/struct derive with the new return type; same pattern as Outer matching child.

## Ordered changes

### P0 — Effect sink; drop `ControlFlow::Break(Output)`

Before: `dispatch -> ControlFlow<Output, Path>`. After: `effs: &mut Vec<Effect>`, always return path-related ascent. Before/after on free `dispatch` and derive recurse as in current `bind` / `bind_macro` (child `?` removed; path always recovered).

### P1 — `Pack` / `AsHere` / `AsUp` / `PeelPack` / `Path` / `after_first_peel` / `complete` / `unpack`

Unit tests: one/two/three peels match nest shapes above (same as `/tmp` proof).

### P2 — `Dispatch::Ascent` + `Claim`; leaf/parent expands as Inner/Outer above

Handlers that kill return `Ascent` via peels + complete. Handlers that only mutate take `&mut focus` on Here arm.

### P3 — Macro emits schedule opts + unpack + after_first_peel

### F1 — `#[post]` Here / Up arms

### F2 — multi-peel kill in exclusive (more `into_parent` before `complete`)

### F3 — `#[pre_post]` snap for Up arm

### F4 — Here-only path mutation posts

## Rules

1. No stubs: every named fn/type has a body.
2. Path = focus + `Pack`. `complete() -> Pack::Out`.
3. `into_parent` peels focus and `peel_pack`s; `Out` unchanged.
4. Doll is nested `Result` inside bind; app uses `Ascent` + `Step`.
5. Same ascent type on every exit from a node.
6. Claim separate.

## Tests

- `after_first_peel` + `complete` → `Ok`
- one `into_parent` + `complete` → `Err(Ok(_))`
- two `into_parent` + `complete` → `Err(Err(Ok(_)))`
- Outer expand KeyA kill / KeyB normal walks
- Claim trap door
