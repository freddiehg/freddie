# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

On the **existing** laserbeam path:

```rust
// path: PathMut<…, PathMut<…, &'a mut Root>>
after_first_peel(path).into_parent().complete()
// → AscentOf<OriginPath> = Doll nest of parent path types
```

Public doll arms are **Here** and **Up**. Not `Ok`/`Err`, not `ControlFlow`.

Spine is `laserbeam::PathMut` and root `&mut T` as today. No parallel `Step` type. Peel is `PathMut::into_parent` / `HasParent::into_parent`.

Deliverable when implemented: unit tests on real `PathMut` nests that assert the doll shapes.

## Existing machinery (do not reinvent)

```rust
// laserbeam
pub struct PathMut<Node, Parent> { /* parent private */ }

impl<Node, Parent> PathMut<Node, Parent> {
    pub fn into_parent(self) -> Parent { self.parent }
    pub fn get(&self) -> &Node { … }
    pub fn get_mut(&mut self) -> &mut Node { … }
    pub fn from_fn(parent, proj_mut, proj_ref) -> Self { … }
}

// bind
pub trait HasParent {
    type Parent;
    fn into_parent(self) -> Self::Parent;
}

impl<N, P> HasParent for laserbeam::PathMut<N, P> {
    type Parent = P;
    fn into_parent(self) -> P {
        PathMut::into_parent(self)
    }
}

pub trait Place {
    type Path<'a> where Self: 'a;
}
// root: Path<'a> = &'a mut Self
// child: Path<'a> = PathMut<Self, Parent::Path<'a>>
```

Place path aliases (as in bind tests / mercury) are already the nest, e.g.:

```rust
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
```

## Public doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop at this path; posts may use it
Up(rest)    — jumped past; rest is the thinner doll
```

```text
Doll<MyPath, Doll<ParentPath, Doll<GrandParentPath, …>>>
```

Name by leave origin (path and/or node as needed):

```rust
pub type AscentOf<P> = <P as AscentOut>::Out;
// type Ascent<'a> = AscentOf<<Self as Place>::Path<'a>>;  // if/when needed
```

## Type family (on real path types)

```rust
/// Doll for a leave that starts at this path type.
pub trait AscentOut {
    type Out;
}

// Root path: &mut T (Place::Path for a root node).
impl<'a, T> AscentOut for &'a mut T {
    type Out = &'a mut T;
}

/// Here(parent) if stop after one peel; Up(parent’s full doll) if go further.
impl<N, P: AscentOut> AscentOut for laserbeam::PathMut<N, P> {
    type Out = Doll<P, P::Out>;
}
```

Unrolling (`NavPath = PathMut<Nav, PathMut<Layer, &mut App>>`):

```text
&mut App::Out
  = &mut App

LayerPath::Out = PathMut<Layer, &mut App>::Out
  = Doll<&mut App, &mut App>

NavPath::Out = PathMut<Nav, LayerPath>::Out
  = Doll<LayerPath, Doll<&mut App, &mut App>>
  = Doll<MyPath, Doll<ParentPath, …>>   // for Layer when Nav returns
```

Posts unpack **child** `AscentOf<ChildPath>`:

```rust
match child_ascent {
    Doll::Here(my_path) => { /* posts; my_path is this level’s PathMut */ }
    Doll::Up(rest) => { /* jumped past this level */ }
}
```

## Pack machine (new; sits on PathMut)

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub trait Pack<P> {
    type Out;
    fn pack(self, path: P) -> Self::Out;
}

pub struct AsHere<E>(PhantomData<E>);

impl<P, E> Pack<P> for AsHere<E> {
    type Out = Doll<P, E>;
    fn pack(self, path: P) -> Doll<P, E> {
        Doll::Here(path)
    }
}

pub struct AsUp<Q, Inner>(Inner, PhantomData<Q>);

impl<Q, Inner, P> Pack<P> for AsUp<Q, Inner>
where
    Inner: Pack<P>,
{
    type Out = Doll<Q, Inner::Out>;
    fn pack(self, path: P) -> Doll<Q, Inner::Out> {
        Doll::Up(self.0.pack(path))
    }
}

pub struct AsTerminal;

impl<P> Pack<P> for AsTerminal {
    type Out = P;
    fn pack(self, path: P) -> P {
        path
    }
}

/// Rewrite pack when focus is PathMut → Parent; Pack::Out unchanged.
pub trait PeelPack<Node, Parent>: Pack<PathMut<Node, Parent>> + Sized {
    type After: Pack<Parent, Out = Self::Out>;
    fn peel_pack(self) -> Self::After;
}

impl<Node, Parent, E> PeelPack<Node, Parent> for AsHere<Doll<Parent, E>> {
    type After = AsUp<PathMut<Node, Parent>, AsHere<E>>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsHere(PhantomData), PhantomData)
    }
}

/// Bare parent rest (root `&mut T`): AsHere<Parent> → Up(parent).
impl<Node, Parent> PeelPack<Node, Parent> for AsHere<Parent> {
    type After = AsUp<PathMut<Node, Parent>, AsTerminal>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsTerminal, PhantomData)
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

/// Leave carrier: laserbeam focus + pack. Not a second path type.
pub struct LeavePath<P, Pk> {
    focus: P,
    pack: Pk,
}

impl<P, Pk> LeavePath<P, Pk>
where
    Pk: Pack<P>,
{
    pub fn complete(self) -> Pk::Out {
        let LeavePath { focus, pack } = self;
        pack.pack(focus)
    }

    pub fn focus(&self) -> &P {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut P {
        &mut self.focus
    }
}

impl<Node, Parent, Pk> LeavePath<PathMut<Node, Parent>, Pk>
where
    Pk: PeelPack<Node, Parent>,
{
    pub fn into_parent(self) -> LeavePath<Parent, Pk::After> {
        let LeavePath { focus, pack } = self;
        LeavePath {
            focus: focus.into_parent(),
            pack: pack.peel_pack(),
        }
    }
}

/// First peel via PathMut::into_parent; Rest = Parent::Out.
pub fn after_first_peel<Node, Parent>(
    path: PathMut<Node, Parent>,
) -> LeavePath<Parent, AsHere<Parent::Out>>
where
    Parent: AscentOut,
{
    LeavePath {
        focus: path.into_parent(),
        pack: AsHere(PhantomData),
    }
}
```

`LeavePath` is only the pack carrier. Focus is always a real `PathMut` or `&mut Root`.

### Concrete `complete()`

```rust
// LeavePath::complete
let LeavePath { focus, pack } = self;
pack.pack(focus)

// AsHere:     Doll::Here(focus)
// AsUp:       Doll::Up(inner.pack(focus))
// AsTerminal: focus
```

### Worked: `PathMut` nest

```rust
// types as in bind tests
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;

// NavPath::Out = Doll<LayerPath, Doll<AppPath, AppPath>>

let nav: NavPath<'_> = PathMut::from_fn(layer_path, |p| &mut p.get_mut().nav, |p| &p.get().nav);

// one peel
let out: AscentOf<NavPath<'_>> = after_first_peel(nav).complete();
// Doll::Here(layer_path)

// two peels
let out: AscentOf<NavPath<'_>> = after_first_peel(nav).into_parent().complete();
// Doll::Up(Doll::Here(app))   // AppPath = &mut App; inner AsHere on &mut App
// or with bare rest after first peel’s Rest shape: see AscentOut for &mut T
```

Bare root rest when `Parent = &mut App` and `AsHere<&mut App>` peels… focus after first peel is already `&mut App`; further peel is not `PathMut`. Two peels from `Nav` land on `AppPath` via `LayerPath::into_parent()`.

```text
after_first_peel(nav)           focus = LayerPath, pack = AsHere<LayerPath::Out>
  .complete()                   Doll::Here(layer)

after_first_peel(nav)
  .into_parent()                focus = AppPath, pack = AsUp<LayerPath, …>
  .complete()                   Doll::Up(…)
```

## Tests (when implemented)

Use the same path aliases as `bind` tests (`App` / `Layer` / `Nav` / `PathMut::from_fn`).

| Case | Assert |
| --- | --- |
| one peel from Nav | `Doll::Here(layer)` |
| two peels from Nav | `Doll::Up(…)` ending at `AppPath` |
| type | assigns to `AscentOf<NavPath<'_>>` |
| `get` still works on `Here` path before further peel | |

```text
cargo test -p bind …   # module TBD; not implemented yet
```

## Ordered changes

### 1 — `Doll`, `AscentOut` for `&mut T` and `PathMut<N, P>`, `AscentOf`

### 2 — `Pack` / `AsHere` / `AsUp` / `AsTerminal` / `PeelPack` / `LeavePath` / `after_first_peel` / `complete`

### 3 — unit tests on real `PathMut` trees from bind test fixtures

No second path type. No `Step`.

## Rules

1. Peel is `laserbeam::PathMut::into_parent` (via `HasParent` where useful).
2. Public arms are `Doll::Here` / `Doll::Up`.
3. `Out` for leave at path type `P` is `<P as AscentOut>::Out`.
4. `LeavePath` = focus (`PathMut` / `&mut T`) + pack only.
5. Name by leave origin path/node as needed; do not spell the nest in signatures.
6. Deliverable is tests on real paths; design settled in this doc first.

## Relation to invalidation

Dispatch leave/kill uses `after_first_peel` / `into_parent` / `complete` on the same `Place::Path` values the derive already builds with `from_fn`. Parent matches `Doll::Here` / `Doll::Up`.
