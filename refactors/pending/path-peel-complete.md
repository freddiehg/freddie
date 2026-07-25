# PathMut peel + complete (Here / Up), on bind’s root / non-root split

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

A child leave decides **at runtime** how many `PathMut::into_parent` peels to do (stop here vs kill further up). Those different depths must **unify into one return type** so the parent can match once:

```rust
match child_leave {
    Doll::Here(path) => { /* … */ }
    Doll::Up(rest) => { /* rest : this node’s own ascent */ }
}
```

That type is the nested `Here`/`Up` doll of `Place::Path` values from the leave origin down to root `&mut T`. Construction is peel history on a real `PathMut` chain — the same paths the bind macro already builds with `from_fn` and recovers with `into_parent`.

Deliverable when implemented: unit tests (including trybuild over-peel and a multi-depth unification test) on the real bind test tree.

## Existing machinery

### Place path types (`bind_macro` `place_impl`)

```rust
// #[node(root)]
type Path<'a> = &'a mut Self;

// #[node(parent = ParentPath)]
type Path<'a> = laserbeam::PathMut<Self, ParentPath<'a>>;
```

Live test tree:

```rust
type AppPath<'a> = &'a mut App;                          // #[node(root)]
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;        // #[node(parent = AppPath)]
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;          // #[node(parent = LayerPath)]
type TypingPath<'a> = PathMut<Typing, LayerPath<'a>>;
type DeepPath<'a> = PathMut<Deep, TypingPath<'a>>;
```

### Edge (descent / recover)

```rust
// child path — always PathMut::from_fn(parent_path, …)
// root parent:    |o| &mut o.field
// non-root parent: |np| &mut np.get_mut().field

// recover — always
child.into_parent()  // PathMut::into_parent → ParentPath
```

Leave peels are that same `into_parent`. No second path ADT.

## Doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop at this Place::Path
Up(rest)    — peeled past; rest is the parent’s ascent (see below)
```

Same shape as `Result` / a two-variant coproduct. Public names stay Here/Up. No frunk.

## Ascent alias recursion (one layer per node)

```text
Ascent(root)       = root’s Place::Path          // &mut App
Ascent(non-root)   = Doll<Self::Path, Ascent(parent)>
```

```rust
// parent is root → bare AppPath
pub type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;

// parent is Layer → nest one Doll
pub type NavAscent<'a> = Doll<NavPath<'a>, LayerAscent<'a>>;
// = Doll<NavPath, Doll<LayerPath, AppPath>>

pub type TypingAscent<'a> = Doll<TypingPath<'a>, LayerAscent<'a>>;
pub type DeepAscent<'a> = Doll<DeepPath<'a>, TypingAscent<'a>>;
```

Macro emission: O(1) per node from `#[node(parent = …)]` / `#[node(root)]`. Do not spell a full depth-d nest by hand for each node.

### What each node receives and returns

```text
Child returns  Ascent(child) = Doll<ChildPath, Ascent(this)>
This matches:
  Here(child_path)  — child’s path; this node’s posts may run; then this leaves as Ascent(this)
  Up(rest)          — rest : Ascent(this) already; return upward unchanged (no rewrap)
This returns   Ascent(this)
```

**App never sees `NavAscent`.** Nav returns `Doll<NavPath, LayerAscent>` to Layer. Layer’s `Up` arm holds `LayerAscent`. App only ever receives `LayerAscent = Doll<LayerPath, AppPath>` — two arms — no matter how deep the leave started.

Root (`#[node(root)]`): no `leave_at` on `&mut App`. Only matches the child’s ascent (for App’s child Layer: `LayerAscent`).

## LeavePath: origin phantom, not wrap nest

The wrap type-state (`Id` / `ComposeUp` / `Terminal`) only re-encoded (origin, peel count). Peel count is determined by (origin path type, focus path type) because the parent chain is linear and each path type appears once. So store the origin; make `into_parent` generic; generate only `complete`.

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

/// `P` = current focus (Place::Path). `Origin` = path type where the leave started.
pub struct LeavePath<P, Origin> {
    focus: P,
    /// fn() -> Origin: covariant-friendly phantom (not PhantomData<Origin> / &'a mut inside).
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

/// ONE impl — all nodes, all depths. No Terminal, no root detection.
impl<N, P, Origin> LeavePath<PathMut<N, P>, Origin> {
    pub fn into_parent(self) -> LeavePath<P, Origin> {
        LeavePath {
            focus: self.focus.into_parent(),
            _origin: PhantomData,
        }
    }
}
```

Peeling past root does not compile: `&mut App` does not unify with `PathMut<N, P>`, so there is no `into_parent` on `LeavePath<AppPath, Origin>`. Error: no method `into_parent` on `LeavePath<&mut App, NavPath<'_>>`.

### `leave_at` / `complete` (generated per origin × focus)

Same count of `complete` bodies as before; no generated `into_parent` per depth.

```rust
pub fn leave_at_nav<'a>(path: NavPath<'a>) -> LeavePath<NavPath<'a>, NavPath<'a>> {
    LeavePath::new(path)
}

// focus = origin
impl<'a> LeavePath<NavPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Here(self.focus)
    }
}

// one peel: focus = LayerPath, origin = NavPath
impl<'a> LeavePath<LayerPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Here(self.focus))
    }
}

// two peels: focus = AppPath, origin = NavPath
impl<'a> LeavePath<AppPath<'a>, NavPath<'a>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Up(self.focus)) // bare root path
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

Macro: for each non-root node X, for each focus type on the chain from X::Path down to root inclusive, emit one `complete` that nests the right number of `Up` and ends in `Here(focus)` or bare root.

Composition view (still true):

```text
complete at origin = Here(path)
each into_parent composes one Up into the finish
complete at focus  = Upⁿ(Here(focus)) or Upⁿ(root) when focus is &mut T
```

The `ⁿ` is now selected by which `(Origin, Focus)` `complete` impl applies, not by a value-level wrap type.

## Composition (function view)

```text
Out = Ascent(origin) fixed for the leave

at origin:  f = id
            complete = Here(focus)

into_parent: f := |x| f(Up(x))     // one layer; type-state collapsed into Origin+Focus

at parent:  complete = Up(Here(focus)) or further Ups per depth
at root:    complete = Up(…(app)) bare
```

Result spelling of Nav leave:

```text
Result<NavPath, Result<LayerPath, AppPath>>
  Ok(nav) / Err(Ok(layer)) / Err(Err(app))
```

## Invalidation sketch

```rust
// child (e.g. Nav) returns NavAscent
match nav_ascent {
    Doll::Here(mut nav_path) => {
        // Nav posts may run (policy: see below)
        // then leave: leave_at_nav is already past; this IS the stop at Nav
        // return upward: this is already NavAscent — parent is Layer
    }
    Doll::Up(layer_ascent) => {
        // layer_ascent : LayerAscent — return to Layer unchanged
    }
}

// Layer receives NavAscent = Doll<NavPath, LayerAscent>
match nav_ascent {
    Doll::Here(nav_path) => { /* … */; /* produce LayerAscent via leave_at_layer / complete */ }
    Doll::Up(layer_ascent) => layer_ascent, // already LayerAscent
}

// App receives only LayerAscent = Doll<LayerPath, AppPath>
match layer_ascent {
    Doll::Here(layer_path) => { /* … */ }
    Doll::Up(app) => { /* app: &mut App */ }
}
```

### Posts on `Up` (deferred)

Whether the node that **receives** `Up(rest)` may run its own posts is **not** fixed here. Types allow either: `rest` is already `Ascent(self)`. Policy belongs in `invalidation.md` (claim / post scheduling). This prefactor only defines peels and the doll.

## PhantomData

Use `PhantomData<fn() -> Origin>` only (see `LeavePath` above). Do not use `PhantomData<NavPath<'a>>` (pulls in `&'a mut` and muddy lifetime/dropck errors).

## Tests

### Keep (macro smoke — constructor placement)

Fixed peel count forces the variant; still useful as expand smoke tests.

```rust
leave_at_nav(nav).complete()                           // Here
leave_at_nav(nav).into_parent().complete()             // Up(Here(_))
leave_at_nav(nav).into_parent().into_parent().complete() // Up(Up(_))
leave_at_layer(layer).into_parent().complete()         // Up(_)
```

### Required: compile-fail over-peel (trybuild)

```rust
// nav_overpeel.rs — must not compile
leave_at_nav(nav).into_parent().into_parent().into_parent();

// layer_overpeel.rs — must not compile
leave_at_layer(layer).into_parent().into_parent();
```

This is the type-state’s whole purchase.

### Required: unification under peel

```rust
fn all_depths<'a>(nav: NavPath<'a>, branch: u8) -> NavAscent<'a> {
    let leave = leave_at_nav(nav);
    match branch {
        0 => leave.complete(),
        1 => leave.into_parent().complete(),
        _ => leave.into_parent().into_parent().complete(),
    }
}
```

If any arm’s type diverges, the ascent alias or a `complete` body is wrong (off-by-one nest).

### Required: path still usable

```rust
// Here(nav): get_mut through focus, assert tree change
// Up(Up(app)): write through recovered &mut App, assert change
```

## Ordered changes

### 1 — `Doll`, `LeavePath<P, Origin>` with generic `into_parent` on `PathMut`

### 2 — Ascent aliases recursive per Place node; `leave_at_*` + `complete` per (origin, focus) for App tree

### 3 — Tests: smoke + trybuild over-peel + unification + mut through recovered paths

### 4 — Macro emission from `#[node(root)]` / `#[node(parent = …)]`

### 5 — invalidation matches `Ascent(child)` / returns `Ascent(self)`; posts-on-Up policy specified there

## Rules

1. Only `Place::Path`: `&mut Root` or `PathMut<Self, ParentPath>`.
2. `Ascent(root) = Path`; `Ascent(node) = Doll<Path, Ascent(parent)>`.
3. `LeavePath<Focus, Origin>`; one generic `into_parent`; generated `complete` only.
4. Child returns `Ascent(child)`; parent returns `Ascent(self)`; `Up` payload at node X is `Ascent(X)`.
5. App sees only `LayerAscent`, never `NavAscent` / `DeepAscent`.
6. Over-peel past root does not compile.
7. Phantom: `fn() -> Origin`. Public arms: Here/Up.
