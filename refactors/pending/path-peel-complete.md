# PathMut peel + complete (Here / Up), on bind’s root / non-root split

Not done. Standalone. Prefactor for `invalidation.md`.

This only makes sense on top of **existing** `Place` paths and the bind derive’s root vs non-root split. It is not a second path system.

Deliverable when implemented: unit tests on the real bind test tree (`App` / `Layer` / `Nav` / `Deep` with `PathMut::from_fn` as the macro emits).

## What bind already does

### Place path types (`bind_macro` `place_impl`)

```rust
// #[node(root)]
impl Place for App {
    type Path<'a> = &'a mut Self;   // NOT PathMut
}

// #[node(parent = LayerPath)]
impl Place for Nav {
    type Path<'a> = laserbeam::PathMut<Self, LayerPath<'a>>;
}
```

From the live test tree:

```rust
// #[node(root)]
type AppPath<'a> = &'a mut App;

// #[node(parent = AppPath)]
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;

// #[node(parent = LayerPath)]
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
type TypingPath<'a> = PathMut<Typing, LayerPath<'a>>;

// #[node(parent = TypingPath)]
type DeepPath<'a> = PathMut<Deep, TypingPath<'a>>;
```

### Descent / ascent today (`derive_support::Edge`)

Parent is root (`is_root: true`) vs non-root changes **projections**, not the fact that the child path is always `PathMut`:

```rust
// Edge::child_path — always PathMut::from_fn(parent_path, …)
// parent_path is either &mut App (root) or PathMut<…> (non-root)

// root parent field:
PathMut::from_fn(path, |o| &mut o.layer, |o| &o.layer)

// non-root parent field:
PathMut::from_fn(path, |np| &mut np.get_mut().deep, |np| &np.get().deep)
```

```rust
// Edge::recover_parent — always the peel we need for leave
child.into_parent()   // PathMut::into_parent → Parent path type
```

So: **every non-root place path is a `PathMut`**. Recovering the parent after a child dispatch is already `into_parent`. This prefactor is that same peel, plus a typed `Here`/`Up` doll for “where did we stop.”

### Root never peels

`&mut App` has no `into_parent`. Root dispatch:

- builds child `PathMut` with `from_fn` on `&mut App`
- on the way back, `recover_parent` yields `&mut App` again
- does **not** run the leave doll **starting at** `&mut App`

Leave/kill dolls start at a **non-root** `Place::Path` (`PathMut<…>`). Root only **receives** a doll whose innermost terminal type is `AppPath` (`&mut App`).

## Public doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop at this path (still a Place::Path value)
Up(rest)    — PathMut::into_parent already moved past that layer
```

## Origin nest = this node’s PathMut parent chain down to root `&mut T`

Leave **started at** `NavPath` (non-root):

```text
NavAscent<'a> =
  Doll<
    NavPath<'a>,                    // Place::Path for Nav = PathMut<Nav, LayerPath>
    Doll<
      LayerPath<'a>,                // Place::Path for Layer = PathMut<Layer, AppPath>
      AppPath<'a>                   // Place::Path for App = &mut App  — bare terminal
    >
  >
```

| peels (`into_parent`) | focus type | `complete` value |
| --- | --- | --- |
| 0 | `NavPath` = `PathMut<Nav, LayerPath>` | `Here(nav)` |
| 1 | `LayerPath` = `PathMut<Layer, AppPath>` | `Up(Here(layer))` |
| 2 | `AppPath` = `&mut App` | `Up(Up(app))` — **bare** `&mut App`, no `Here(app)` |

Leave **started at** `LayerPath` (parent is root):

```text
LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>

0 peels:  Here(layer)
1 peel:   Up(app)          // Terminal — parent is &mut App
```

Leave **started at** `DeepPath`:

```text
DeepAscent<'a> =
  Doll<DeepPath, Doll<TypingPath, Doll<LayerPath, AppPath>>>
// four Place path types; last is still bare AppPath
```

Same rule for every non-root node: nest is `Doll<Self::Path, <parent path nest>>` until root `&mut T` is bare at the bottom.

## Function `f` on the leave (peel history)

```text
complete (non-root focus) = f(Here(focus))
complete (root &mut T)    = bare Up chain ending in focus   // Terminal
```

```text
start at NavPath:     f = id
                      complete → Here(nav)

PathMut::into_parent  // same as Edge::recover_parent
f := |x| f(Up(x))
                      complete → Up(Here(layer))

PathMut::into_parent  // LayerPath → AppPath; parent is root
f := |x| f(Up(x)) with Terminal
                      complete → Up(Up(app))
```

Result spelling:

```text
Result<NavPath, Result<LayerPath, AppPath>>
  Ok(nav)         = Here(nav)
  Err(Ok(layer))  = Up(Here(layer))
  Err(Err(app))   = Up(Up(app))
```

No `!`. Root path type is the terminal `U` of the last `Doll` / `Err`.

## Data structures

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub enum Doll<H, U> {
    Here(H),
    Up(U),
}

pub struct Id<Rest>(PhantomData<Rest>);

impl<Rest> Id<Rest> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

pub struct ComposeUp<Skipped, Inner> {
    inner: Inner,
    _skipped: PhantomData<Skipped>,
}

impl<Skipped, Inner> ComposeUp<Skipped, Inner> {
    pub const fn new(inner: Inner) -> Self {
        Self {
            inner,
            _skipped: PhantomData,
        }
    }
}

/// Innermost wrap when focus is root Place::Path (&mut T).
pub struct Terminal;

/// `P` is always a Place::Path: PathMut<…> or &mut Root.
pub struct LeavePath<P, F> {
    focus: P,
    f: F,
}
```

`ComposeUp` / `Id` / `Terminal` are the type-state for `f` (composed `Up` constructors). One `PathMut::into_parent` ↔ one `ComposeUp` layer.

## Full impls for the bind test tree

### Nav (`#[node(parent = LayerPath)]`)

```rust
pub type NavAscent<'a> = Doll<NavPath<'a>, Doll<LayerPath<'a>, AppPath<'a>>>;

pub fn leave_at_nav<'a>(path: NavPath<'a>) -> LeavePath<NavPath<'a>, Id<Doll<LayerPath<'a>, AppPath<'a>>>> {
    LeavePath {
        focus: path,
        f: Id::new(),
    }
}

impl<'a> LeavePath<NavPath<'a>, Id<Doll<LayerPath<'a>, AppPath<'a>>>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Here(self.focus)
    }

    pub fn into_parent(self) -> LeavePath<LayerPath<'a>, ComposeUp<NavPath<'a>, Id<AppPath<'a>>>> {
        LeavePath {
            focus: self.focus.into_parent(), // PathMut::into_parent → LayerPath
            f: ComposeUp::new(Id::new()),
        }
    }
}

impl<'a> LeavePath<LayerPath<'a>, ComposeUp<NavPath<'a>, Id<AppPath<'a>>>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Here(self.focus))
    }

    /// Next parent is AppPath = &mut App → Terminal
    pub fn into_parent(
        self,
    ) -> LeavePath<AppPath<'a>, ComposeUp<NavPath<'a>, ComposeUp<LayerPath<'a>, Terminal>>> {
        LeavePath {
            focus: self.focus.into_parent(), // → &mut App
            f: ComposeUp::new(ComposeUp::new(Terminal)),
        }
    }
}

impl<'a> LeavePath<AppPath<'a>, ComposeUp<NavPath<'a>, ComposeUp<LayerPath<'a>, Terminal>>> {
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Up(self.focus))
    }
}
```

### Layer (`#[node(parent = AppPath)]` — parent is root)

```rust
pub type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;

pub fn leave_at_layer<'a>(path: LayerPath<'a>) -> LeavePath<LayerPath<'a>, Id<AppPath<'a>>> {
    LeavePath {
        focus: path,
        f: Id::new(),
    }
}

impl<'a> LeavePath<LayerPath<'a>, Id<AppPath<'a>>> {
    pub fn complete(self) -> LayerAscent<'a> {
        Doll::Here(self.focus)
    }

    pub fn into_parent(self) -> LeavePath<AppPath<'a>, ComposeUp<LayerPath<'a>, Terminal>> {
        LeavePath {
            focus: self.focus.into_parent(), // PathMut → &mut App
            f: ComposeUp::new(Terminal),
        }
    }
}

impl<'a> LeavePath<AppPath<'a>, ComposeUp<LayerPath<'a>, Terminal>> {
    pub fn complete(self) -> LayerAscent<'a> {
        Doll::Up(self.focus)
    }
}
```

### Deep (`#[node(parent = TypingPath)]`)

Same pattern; nest has four layers; last peel still ends in `Terminal` + `AppPath`. Emit with a macro once Nav/Layer tests pass:

```text
DeepAscent = Doll<DeepPath, Doll<TypingPath, Doll<LayerPath, AppPath>>>
```

### App (`#[node(root)]`)

No `leave_at_app`. Root path is `&mut App`. After a child returns `LayerAscent` / `NavAscent`, root matches `Here`/`Up` and finishes. Free `dispatch` only needs effects + claim.

## Tie-in to current dispatch recover

Today (macro expand):

```rust
let child = Child::dispatch(child_path, event)?;
path = child.into_parent(); // Edge::recover_parent
```

After this prefactor (invalidation):

```rust
let ascent = Child::leave_or_dispatch(…); // returns NavAscent / LayerAscent / …
match ascent {
    Doll::Here(mut path) => {
        // posts at this Place::Path
        // leave: leave_at_*(path).into_parent()*.complete() or continue doll
    }
    Doll::Up(rest) => { /* posts dropped; rest is thinner Place path nest */ }
}
```

`into_parent` on `LeavePath` is the same `PathMut::into_parent` recover already uses; the wrap type-state is the only addition.

## Root vs non-root (checklist)

| | `#[node(root)]` | `#[node(parent = …)]` |
| --- | --- | --- |
| `Place::Path` | `&mut Self` | `PathMut<Self, ParentPath>` |
| `from_fn` parent | `&mut Root` field/variant | `path.get_mut()` / `path.get()` |
| `into_parent` on path | no | yes → `ParentPath` |
| start leave doll | no | yes (`leave_at_*`) |
| appears in doll | bare terminal `AppPath` only | `Here` or intermediate `Up(Here(…))` |
| detect Terminal peel | — | `ParentPath` is `&mut T` not `PathMut` |

## Tests (deliverable)

Use `bind` fixtures: build `App`, take `LayerPath` / `NavPath` via the same `from_fn` the macro emits (or call into a test helper that mirrors `Edge::child_path`).

```rust
#[test]
fn nav_here() {
    // nav: NavPath from real App tree
    assert!(matches!(leave_at_nav(nav).complete(), Doll::Here(_)));
}

#[test]
fn nav_up_here_layer() {
    assert!(matches!(
        leave_at_nav(nav).into_parent().complete(),
        Doll::Up(Doll::Here(_))
    ));
}

#[test]
fn nav_up_up_app() {
    assert!(matches!(
        leave_at_nav(nav).into_parent().into_parent().complete(),
        Doll::Up(Doll::Up(_))
    ));
}

#[test]
fn layer_up_app() {
    assert!(matches!(
        leave_at_layer(layer).into_parent().complete(),
        Doll::Up(_)
    ));
}

#[test]
fn types_are_place_paths() {
    let out: NavAscent<'_> = leave_at_nav(nav).into_parent().into_parent().complete();
    let _: &mut App = match out {
        Doll::Up(Doll::Up(app)) => app,
        _ => panic!(),
    };
}
```

## Ordered changes

### 1 — `Doll`, `Id`, `ComposeUp`, `Terminal`, `LeavePath` in bind

### 2 — `leave_at_nav` + `leave_at_layer` + tests on real `App` tree paths

### 3 — Macro: given `Place` path nest (root vs `PathMut` layers), emit `leave_at_*` / ascent alias for that node

### 4 — invalidation: child returns ascent doll; parent matches `Here`/`Up`; recover = same `into_parent`

## Rules

1. Only `Place::Path` values: `&mut Root` or `PathMut<Self, ParentPath>`.
2. Peel is only `PathMut::into_parent` (same as `Edge::recover_parent`).
3. Leave dolls start only on non-root places; root is bare terminal inside the nest.
4. `complete` at `PathMut` focus: `f(Here(focus))`; at `&mut Root`: bare `Up` chain + `Terminal`.
5. Public match: `Doll::Here` / `Doll::Up`.
6. Tests use the existing bind tree and path aliases.
