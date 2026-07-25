# PathMut peel + complete (Here / Up)

Not done. Standalone. Prefactor for `invalidation.md`.

## What

Runtime peel depth unifies to one return type. Parent matches once:

```rust
match child_leave {
    Doll::Here(path) => { /* path: child’s Place::Path */ }
    Doll::Up(rest) => { /* rest: Ascent(this node) */ }
}
```

Peel is `PathMut::into_parent` on existing `Place::Path` values.

`(Focus, Origin)` alone selects the nest constructor: a `Place::Path` type encodes its full ancestry; each step up is a structural suffix of the previous type; a focus type appears at most once on the chain from any origin. `PathMut<N, PathMut<N, _>>` is still a different type from `PathMut<N, _>`, so `(Focus, Origin)` stays unambiguous.

## Place paths (bind macro)

```rust
// #[node(root)]
type Path<'a> = &'a mut Self;

// #[node(parent = ParentPath)]
type Path<'a> = laserbeam::PathMut<Self, ParentPath<'a>>;
```

```rust
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
type TypingPath<'a> = PathMut<Typing, LayerPath<'a>>;
type DeepPath<'a> = PathMut<Deep, TypingPath<'a>>;
```

```rust
// Edge::child_path
PathMut::from_fn(parent_path, project, project_ref)

// Edge::recover_parent
child.into_parent()
```

## Doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop at this Place::Path
Up(rest)    — peeled past; rest is Ascent(parent) = Ascent(this) when matched at this node
```

## Ascent aliases

```text
Ascent(root)     = Place::Path of root
Ascent(non-root) = Doll<Self::Path, Ascent(parent)>
```

```rust
pub type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;
pub type NavAscent<'a> = Doll<NavPath<'a>, LayerAscent<'a>>;
pub type TypingAscent<'a> = Doll<TypingPath<'a>, LayerAscent<'a>>;
pub type DeepAscent<'a> = Doll<DeepPath<'a>, TypingAscent<'a>>;
```

Macro (consumer): emit one alias per place node from `#[node(root)]` / `#[node(parent = …)]`.

```text
Child returns  Ascent(child) = Doll<ChildPath, Ascent(this)>
This matches   Here(child_path) | Up(rest)   // rest: Ascent(this)
This returns   Ascent(this)
```

```text
Nav  → Layer : NavAscent
Layer → App  : LayerAscent
App receives only LayerAscent
```

Root: no `leave_at` on `&mut App`. Root matches child-of-root ascent only.

## LeavePath

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub struct LeavePath<P, Origin> {
    focus: P,
    _origin: PhantomData<fn() -> Origin>,
}

impl<P, Origin> LeavePath<P, Origin> {
    pub fn new(focus: P) -> Self {
        Self {
            focus,
            _origin: PhantomData,
        }
    }

    pub fn focus(&self) -> &P {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut P {
        &mut self.focus
    }
}

/// All nodes, all depths.
impl<N, P, Origin> LeavePath<PathMut<N, P>, Origin> {
    pub fn into_parent(self) -> LeavePath<P, Origin> {
        LeavePath {
            focus: self.focus.into_parent(),
            _origin: PhantomData,
        }
    }
}

pub fn leave<P>(path: P) -> LeavePath<P, P> {
    LeavePath::new(path)
}
```

`into_parent` only on `PathMut`. `LeavePath<&mut R, Origin>` has no `into_parent` — over-peel past root does not compile.

## `complete` in bind (by path structure, not by node name)

Consumer crates cannot write inherent `impl LeavePath<NavPath, NavPath>` (E0116: inherent impl on foreign type). `complete` lives in **bind**, generic over node/root type parameters, one impl per **(origin depth, focus position)**.

Return types are the structural expansions of the Ascent equations. Consumer aliases (`NavAscent`, …) name those same types.

### Depth 1 origin (child of root): `PathMut<N, &mut R>`

```rust
// focus = origin
impl<'a, N, R> LeavePath<PathMut<N, &'a mut R>, PathMut<N, &'a mut R>> {
    pub fn complete(self) -> Doll<PathMut<N, &'a mut R>, &'a mut R> {
        Doll::Here(self.focus)
    }
}

// focus = root
impl<'a, N, R> LeavePath<&'a mut R, PathMut<N, &'a mut R>> {
    pub fn complete(self) -> Doll<PathMut<N, &'a mut R>, &'a mut R> {
        Doll::Up(self.focus)
    }
}
```

`LayerAscent<'a>` = that return type with `N = Layer`, `R = App`.

### Depth 2 origin: `PathMut<N1, PathMut<N2, &mut R>>`

```rust
type P2<'a, N1, N2, R> = PathMut<N1, PathMut<N2, &'a mut R>>;
type P1<'a, N2, R> = PathMut<N2, &'a mut R>;

// focus = origin
impl<'a, N1, N2, R> LeavePath<P2<'a, N1, N2, R>, P2<'a, N1, N2, R>> {
    pub fn complete(
        self,
    ) -> Doll<P2<'a, N1, N2, R>, Doll<P1<'a, N2, R>, &'a mut R>> {
        Doll::Here(self.focus)
    }
}

// focus = parent (one peel)
impl<'a, N1, N2, R> LeavePath<P1<'a, N2, R>, P2<'a, N1, N2, R>> {
    pub fn complete(
        self,
    ) -> Doll<P2<'a, N1, N2, R>, Doll<P1<'a, N2, R>, &'a mut R>> {
        Doll::Up(Doll::Here(self.focus))
    }
}

// focus = root (two peels)
impl<'a, N1, N2, R> LeavePath<&'a mut R, P2<'a, N1, N2, R>> {
    pub fn complete(
        self,
    ) -> Doll<P2<'a, N1, N2, R>, Doll<P1<'a, N2, R>, &'a mut R>> {
        Doll::Up(Doll::Up(self.focus))
    }
}
```

`NavAscent<'a>` = that return type with `N1 = Nav`, `N2 = Layer`, `R = App`.

### Depth 3 and 4

Same pattern: origin depth d needs d+1 `complete` impls (focus at origin, each parent, root). Depths 1..4 (Deep): 2+3+4+5 = 14 impls in bind, once for every tree.

`&mut R` never unifies with `PathMut<_, _>` in the parent slot → impls stay disjoint.

### Consumer surface

```rust
// bind
pub fn leave<P>(path: P) -> LeavePath<P, P> { LeavePath::new(path) }

// consumer / derive — aliases only (legal)
pub type NavAscent<'a> = Doll<NavPath<'a>, LayerAscent<'a>>;
pub type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;

// optional thin wrappers
pub fn leave_at_nav<'a>(path: NavPath<'a>) -> LeavePath<NavPath<'a>, NavPath<'a>> {
    leave(path)
}
```

No inherent `complete` emitted into mercury. No private `LeavePath::new` required from outside if `leave` / `LeavePath::new` is public.

## Parent match

```rust
match nav_ascent {
    Doll::Here(nav_path) => { /* … produce LayerAscent */ }
    Doll::Up(layer_ascent) => layer_ascent,
}

match layer_ascent {
    Doll::Here(layer_path) => { /* … */ }
    Doll::Up(app) => { /* &mut App */ }
}
```

Posts when receiving `Up`: `invalidation.md`.

## Tests

```rust
// smoke (constructor placement)
leave(nav).complete();
leave(nav).into_parent().complete();
leave(nav).into_parent().into_parent().complete();
leave(layer).into_parent().complete();

// trybuild — must not compile
leave(nav).into_parent().into_parent().into_parent();
leave(layer).into_parent().into_parent();

// unification — nest invariant under peel count
fn all_depths<'a>(nav: NavPath<'a>, branch: u8) -> NavAscent<'a> {
    let leave = leave(nav);
    match branch {
        0 => leave.complete(),
        1 => leave.into_parent().complete(),
        _ => leave.into_parent().into_parent().complete(),
    }
}

// usable paths: mutate via Here(nav) get_mut; mutate via Up(Up(app))
```

## Ordered changes

### 1 — `Doll`, `LeavePath<P, Origin>`, generic `into_parent`, public `leave`

### 2 — Depth-keyed `complete` impls in bind (depths 1..4)

### 3 — Tests on App tree paths: smoke, trybuild over-peel, unification, mut through recovered paths

### 4 — Derive emits ascent aliases (+ optional `leave_at_*` thin wrappers over `leave`). No inherent impls on `LeavePath` in the consumer.

### 5 — invalidation: match `Ascent(child)`, return `Ascent(self)`; posts on `Up`

## Rules

1. `Place::Path` only: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Ascent(root) = Path`; `Ascent(node) = Doll<Path, Ascent(parent)>`.
3. `LeavePath<Focus, Origin>`; one `into_parent` on `PathMut`; `complete` only as bind inherent impls keyed by path structure/depth.
4. Child returns `Ascent(child)`; parent returns `Ascent(self)`; `Up` payload is `Ascent(self)`.
5. App receives only `LayerAscent`.
6. Over-peel past root does not compile.
7. `_origin: PhantomData<fn() -> Origin>`. Arms: `Here` / `Up`.
