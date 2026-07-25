# PathMut peel + complete (Here / Up)

Not done. Standalone. Prefactor for `invalidation.md`.

## What

The leave API is on the path values themselves. No wrapper type, no constructor:

```text
path.complete()                                // Here(path)
path.into_parent().complete()                  // Up(Here(parent))
path.into_parent().into_parent().complete()    // Up(Up(root))
```

Peel is laserbeam `PathMut::into_parent` on existing `Place::Path` values. `complete` is bind's `Complete<A>` trait method; `A` is the origin's ascent, pinned by the dispatch return type.

Runtime peel depth unifies to one return type. Parent matches once:

```rust
match child_leave {
    Doll::Here(path) => { /* path: child’s Place::Path */ }
    Doll::Up(rest) => { /* rest: Ascent(this node) */ }
}
```

`(Focus, A)` alone selects the nest constructor: a `Place::Path` type encodes its full ancestry; each step up is a structural suffix of the previous type; a focus type appears at most once in any ascent nest. `PathMut<N, PathMut<N, _>>` is still a different type from `PathMut<N, _>`, so `(Focus, A)` stays unambiguous, and the `Up` count is determined by the pair.

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

Macro (consumer): emit one alias per place node from `#[node(root)]` / `#[node(parent = …)]`. That is all the derive emits for this feature.

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

Root has no ascent of its own. Root matches the child-of-root ascent only.

## Complete

```rust
pub trait Complete<A> {
    fn complete(self) -> A;
}
```

- `A` is a type parameter, not an associated type: one focus completes into every ascent whose chain contains it (`LayerPath` into `NavAscent`, `TypingAscent`, `DeepAscent`, `LayerAscent`).
- A trait is what makes this API expressible at all: `PathMut` is laserbeam's type and the root path is `&mut Root`, so no crate can put an inherent `complete` on both. The trait replaces `LeavePath`'s `Origin` phantom; the expected return type carries the origin.
- The dispatch return type pins `A`. A call outside return position needs an annotation.
- Call sites need `Complete` in scope; generated dispatch imports it.

## `complete` impls in bind (by path structure, not by node name)

All impls live in bind, generic over node/root type parameters, one impl per **(origin depth, focus position)**. The consumer emits nothing. Return types are the structural expansions of the Ascent equations; consumer aliases (`NavAscent`, …) name those same types.

### Depth 1 origin (child of root)

```rust
type Ascent1<'a, N, R> = Doll<PathMut<N, &'a mut R>, &'a mut R>;

// focus = origin
impl<'a, N, R> Complete<Ascent1<'a, N, R>> for PathMut<N, &'a mut R> {
    fn complete(self) -> Ascent1<'a, N, R> {
        Doll::Here(self)
    }
}

// focus = root
impl<'a, N, R> Complete<Ascent1<'a, N, R>> for &'a mut R {
    fn complete(self) -> Ascent1<'a, N, R> {
        Doll::Up(self)
    }
}
```

`LayerAscent<'a>` = `Ascent1` with `N = Layer`, `R = App`.

### Depth 2 origin

```rust
type P1<'a, N2, R> = PathMut<N2, &'a mut R>;
type P2<'a, N1, N2, R> = PathMut<N1, P1<'a, N2, R>>;
type Ascent2<'a, N1, N2, R> = Doll<P2<'a, N1, N2, R>, Doll<P1<'a, N2, R>, &'a mut R>>;

// focus = origin
impl<'a, N1, N2, R> Complete<Ascent2<'a, N1, N2, R>> for P2<'a, N1, N2, R> {
    fn complete(self) -> Ascent2<'a, N1, N2, R> {
        Doll::Here(self)
    }
}

// focus = parent (one peel)
impl<'a, N1, N2, R> Complete<Ascent2<'a, N1, N2, R>> for P1<'a, N2, R> {
    fn complete(self) -> Ascent2<'a, N1, N2, R> {
        Doll::Up(Doll::Here(self))
    }
}

// focus = root (two peels)
impl<'a, N1, N2, R> Complete<Ascent2<'a, N1, N2, R>> for &'a mut R {
    fn complete(self) -> Ascent2<'a, N1, N2, R> {
        Doll::Up(Doll::Up(self))
    }
}
```

`NavAscent<'a>` = `Ascent2` with `N1 = Nav`, `N2 = Layer`, `R = App`.

### Depth 3 and 4

Same pattern: origin depth d needs d+1 impls (focus at origin, each parent, root). Depths 1..4 (Deep): 2+3+4+5 = 14 impls in bind, once for every tree.

Disjointness: impls for the same `A` have foci at distinct chain positions, and `&mut R` never unifies with `PathMut<_, _>`; impls for the same focus have structurally different `A` nests. Off-chain completes have no impl (`NavPath` into `LayerAscent` does not compile).

### Call sites

```text
// in Nav dispatch (return type NavAscent)
path.complete()                                // Here(nav)
path.into_parent().complete()                  // Up(Here(layer))
path.into_parent().into_parent().complete()    // Up(Up(app))

// in Layer dispatch (return type LayerAscent)
path.complete()                                // Here(layer)
path.into_parent().complete()                  // Up(app)
```

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
// smoke (annotation plays the dispatch return type)
let _: NavAscent<'_> = nav.complete();
let _: NavAscent<'_> = nav.into_parent().complete();
let _: NavAscent<'_> = nav.into_parent().into_parent().complete();
let _: LayerAscent<'_> = layer.into_parent().complete();

// trybuild — must not compile
nav.into_parent().into_parent().into_parent(); // no into_parent on &mut App
let _: LayerAscent<'_> = nav.complete();       // off-chain: no impl

// unification — one return type across peel depths
fn all_depths<'a>(nav: NavPath<'a>, branch: u8) -> NavAscent<'a> {
    match branch {
        0 => nav.complete(),
        1 => nav.into_parent().complete(),
        _ => nav.into_parent().into_parent().complete(),
    }
}

// usable paths: mutate via Here(nav) get_mut; mutate via Up(Up(app))
```

## Ordered changes

### 1 — `Doll`, `trait Complete<A>` in bind

### 2 — Depth-keyed `Complete` impls in bind (depths 1..4)

### 3 — Tests on App tree paths: smoke, trybuild over-peel + off-chain complete, unification, mut through recovered paths

### 4 — Derive emits ascent aliases only

### 5 — invalidation: match `Ascent(child)`, return `Ascent(self)`; posts on `Up`

## Rules

1. `Place::Path` only: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Ascent(root) = Path`; `Ascent(node) = Doll<Path, Ascent(parent)>`.
3. Peel is laserbeam `PathMut::into_parent` only; no wrapper type.
4. `complete` is `Complete<A>::complete`, impls only in bind, keyed by path structure/depth; `A` pinned by the dispatch return type.
5. Child returns `Ascent(child)`; parent returns `Ascent(self)`; `Up` payload is `Ascent(self)`.
6. App receives only `LayerAscent`.
7. Over-peel past root and off-chain `complete` do not compile.
8. Arms: `Here` / `Up`.
