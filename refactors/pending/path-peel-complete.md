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

## Ascent aliases

```text
Ascent(root)     = Place::Path of root          // AppPath
Ascent(non-root) = Doll<Self::Path, Ascent(parent)>
```

```rust
pub type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;
pub type NavAscent<'a> = Doll<NavPath<'a>, LayerAscent<'a>>;
pub type TypingAscent<'a> = Doll<TypingPath<'a>, LayerAscent<'a>>;
pub type DeepAscent<'a> = Doll<DeepPath<'a>, TypingAscent<'a>>;
```

```text
Child returns  Ascent(child) = Doll<ChildPath, Ascent(this)>
This matches   Here(child_path) | Up(rest)   // rest: Ascent(this)
This returns   Ascent(this)
```

```text
Nav  → Layer : NavAscent   = Doll<NavPath, LayerAscent>
Layer → App  : LayerAscent = Doll<LayerPath, AppPath>
App receives only LayerAscent
```

Root has no `leave_at` on `&mut App`. Root matches the child-of-root ascent only.

## LeavePath

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub struct LeavePath<P, Origin> {
    focus: P,
    _origin: PhantomData<fn() -> Origin>,
}

impl<P, Origin> LeavePath<P, Origin> {
    fn new(focus: P) -> Self {
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

impl<N, P, Origin> LeavePath<PathMut<N, P>, Origin> {
    pub fn into_parent(self) -> LeavePath<P, Origin> {
        LeavePath {
            focus: self.focus.into_parent(),
            _origin: PhantomData,
        }
    }
}
```

## leave_at + complete

```rust
pub fn leave_at_nav<'a>(path: NavPath<'a>) -> LeavePath<NavPath<'a>, NavPath<'a>> {
    LeavePath::new(path)
}

impl<'a> LeavePath<NavPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Here(self.focus)
    }
}

impl<'a> LeavePath<LayerPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Here(self.focus))
    }
}

impl<'a> LeavePath<AppPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Up(self.focus))
    }
}

pub fn leave_at_layer<'a>(path: LayerPath<'a>) -> LeavePath<LayerPath<'a>, LayerPath<'a>> {
    LeavePath::new(path)
}

impl<'a> LeavePath<LayerPath<'a>, LayerPath<'a>> {
    pub fn complete(self) -> LayerAscent<'a> {
        Doll::Here(self.focus)
    }
}

impl<'a> LeavePath<AppPath<'a>, LayerPath<'a>> {
    pub fn complete(self) -> LayerAscent<'a> {
        Doll::Up(self.focus)
    }
}
```

Macro: for each non-root place X, for each focus on the chain from `X::Path` down to root inclusive, one `complete` impl returning `Ascent(X)`.

```text
leave_at_nav(nav).complete()                              // Here(nav)
leave_at_nav(nav).into_parent().complete()                // Up(Here(layer))
leave_at_nav(nav).into_parent().into_parent().complete()  // Up(Up(app))

leave_at_layer(layer).complete()                          // Here(layer)
leave_at_layer(layer).into_parent().complete()            // Up(app)
```

## Parent match

```rust
match nav_ascent {
    Doll::Here(nav_path) => {
        // …
        // return a LayerAscent (leave from layer, or construct from path)
    }
    Doll::Up(layer_ascent) => layer_ascent,
}

match layer_ascent {
    Doll::Here(layer_path) => { /* … */ }
    Doll::Up(app) => { /* app: &mut App */ }
}
```

Posts when receiving `Up`: specified in `invalidation.md`.

## Tests

```rust
// smoke
leave_at_nav(nav).complete();
leave_at_nav(nav).into_parent().complete();
leave_at_nav(nav).into_parent().into_parent().complete();
leave_at_layer(layer).into_parent().complete();

// trybuild: must not compile
leave_at_nav(nav).into_parent().into_parent().into_parent();
leave_at_layer(layer).into_parent().into_parent();

// unification
fn all_depths<'a>(nav: NavPath<'a>, branch: u8) -> NavAscent<'a> {
    let leave = leave_at_nav(nav);
    match branch {
        0 => leave.complete(),
        1 => leave.into_parent().complete(),
        _ => leave.into_parent().into_parent().complete(),
    }
}

// usable paths: mutate through Here(nav).focus get_mut; mutate through Up(Up(app))
```

## Ordered changes

### 1 — `Doll`, `LeavePath<P, Origin>`, generic `into_parent`

### 2 — Ascent aliases; `leave_at_*` + `complete` for App tree

### 3 — Tests: smoke, trybuild over-peel, unification, mut through recovered paths

### 4 — Macro from `#[node(root)]` / `#[node(parent = …)]`

### 5 — invalidation: match `Ascent(child)`, return `Ascent(self)`; posts on `Up`

## Rules

1. `Place::Path` only: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Ascent(root) = Path`; `Ascent(node) = Doll<Path, Ascent(parent)>`.
3. `LeavePath<Focus, Origin>`; one `into_parent` on `PathMut`; generated `complete`.
4. Child returns `Ascent(child)`; parent returns `Ascent(self)`; `Up` payload is `Ascent(self)`.
5. App receives only `LayerAscent`.
6. Over-peel past root does not compile.
7. `_origin: PhantomData<fn() -> Origin>`. Arms: `Here` / `Up`.
