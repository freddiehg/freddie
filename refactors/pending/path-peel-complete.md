# PathMut peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

Leave starts on a **`laserbeam::PathMut`**. Peel is only `PathMut::into_parent`. The doll is nested `Here`/`Up` of the **parent path types** along that `PathMut` chain (ending at root `&mut T`).

Deliverable when implemented: unit tests on real `PathMut::from_fn` trees that assert the nests.

## Existing machinery (the only path type)

```rust
// crates/laserbeam
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
    shared: ProjRef<Node, Parent>,
}

impl<Node, Parent> PathMut<Node, Parent> {
    pub const fn from_fn(
        parent: Parent,
        projection: fn(&mut Parent) -> &mut Node,
        shared: fn(&Parent) -> &Node,
    ) -> Self { /* … */ }

    pub fn get(&self) -> &Node { /* … */ }
    pub fn get_mut(&mut self) -> &mut Node { /* … */ }
    pub const fn parent(&self) -> &Parent { /* … */ }

    /// The only peel.
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}

// crates/bind
impl<N, P> HasParent for laserbeam::PathMut<N, P> {
    type Parent = P;
    fn into_parent(self) -> P {
        laserbeam::PathMut::into_parent(self)
    }
}

pub trait Place {
    type Path<'a>
    where
        Self: 'a;
}
// root:  type Path<'a> = &'a mut Self;
// child: type Path<'a> = PathMut<Self, <Parent as Place>::Path<'a>>;
```

### Real path nests (bind / mercury style)

```rust
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
type DeepPath<'a> = PathMut<Deep, NavPath<'a>>;
```

Every leave example below is one of these (or the same shape).

## Doll

```rust
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

Same shape as `Result` / frunk `Coproduct` (`Inl`/`Inr`). Public names stay **Here** / **Up**.

```text
Here(path)  — stop at this PathMut (or root path)
Up(rest)    — PathMut::into_parent already happened past that layer
```

## Origin nest is the PathMut parent chain

Leave started **at** `NavPath` (includes `NavPath` as outermost Here):

```text
NavAscent<'a> =
  Doll<
    NavPath<'a>,
    Doll<
      LayerPath<'a>,
      AppPath<'a>           // root path bare — last Up holds &mut App
    >
  >
```

| stop after peels | `PathMut` focus | value |
| --- | --- | --- |
| 0 | `NavPath` | `Here(nav)` |
| 1× `into_parent` | `LayerPath` | `Up(Here(layer))` |
| 2× `into_parent` | `AppPath` (`&mut App`) | `Up(Up(app))` — **bare** app, no `Here(app)` |

```text
// Result spelling of the same nest:
Result<NavPath, Result<LayerPath, AppPath>>
  Ok(nav)              = Here(nav)
  Err(Ok(layer))       = Up(Here(layer))
  Err(Err(app))        = Up(Up(app))
```

No `Result<AppPath, !>` / `CNil` unless we choose uniform termination later. Prefer bare `AppPath` as the nest bottom (matches root `Place::Path = &mut Self`).

## Function on every leave: `complete = f(Here(path_mut_or_root))`

`LeavePath` wraps a **focus** that is either `PathMut<…>` or root `&mut T`, plus type-state for `f`:

```text
start at NavPath:     f = id
complete:             f(Here(nav)) = Here(nav)

PathMut::into_parent  // NavPath → LayerPath
f := |x| f(Up(x))     // one Up per peel
complete:             f(Here(layer)) = Up(Here(layer))

PathMut::into_parent  // LayerPath → AppPath
f := |x| f(Up(x))     // compose
complete at root:     Up(Up(app))   // Terminal: no Here on &mut App
```

One-level lift (Result language), still about **path types**:

```text
// lift past NavPath layer:
Result<LayerPath, T> → Result<NavPath, Result<LayerPath, T>>   // Err / Up

f_layer = lift_nav ∘ Here     // |layer| Err(Ok(layer))
f_app   = lift_nav ∘ lift_layer  // bare app: |app| Err(Err(app))
```

## Data structures

Focus is always `PathMut` or root path. Wrap is the composed `f` (Cayley / difference-list of `Up` constructors — one per `PathMut::into_parent`).

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub enum Doll<H, U> {
    Here(H),
    Up(U),
}

/// f = id.
pub struct Id<Rest>(PhantomData<Rest>);

impl<Rest> Id<Rest> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

/// f = |x| outer(Up(x)). `Skipped` = the PathMut type just peeled.
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

/// Innermost at root `&mut T`: Up(app) with no Here.
pub struct Terminal;

/// `P` is PathMut<…> or &mut Root.
pub struct LeavePath<P, F> {
    focus: P,
    f: F,
}
```

## Full impls: leave started at `NavPath`

```rust
// Types (production lifetimes)
// type AppPath<'a> = &'a mut App;
// type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
// type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
// type NavAscent<'a> = Doll<NavPath<'a>, Doll<LayerPath<'a>, AppPath<'a>>>;

pub type NavAscent<'a> = Doll<NavPath<'a>, Doll<LayerPath<'a>, AppPath<'a>>>;

pub fn leave_at_nav<'a>(path: NavPath<'a>) -> LeavePath<NavPath<'a>, Id<Doll<LayerPath<'a>, AppPath<'a>>>> {
    LeavePath {
        focus: path,
        f: Id::new(),
    }
}

impl<'a> LeavePath<NavPath<'a>, Id<Doll<LayerPath<'a>, AppPath<'a>>>> {
    /// f = id → Here(nav)
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Here(self.focus)
    }

    /// PathMut::into_parent: NavPath → LayerPath; f := Up
    pub fn into_parent(self) -> LeavePath<LayerPath<'a>, ComposeUp<NavPath<'a>, Id<AppPath<'a>>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp::new(Id::new()),
        }
    }
}

impl<'a> LeavePath<LayerPath<'a>, ComposeUp<NavPath<'a>, Id<AppPath<'a>>>> {
    /// f(Here(layer)) = Up(Here(layer))
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Here(self.focus))
    }

    /// PathMut::into_parent: LayerPath → AppPath; f := Up∘Up, Terminal at root
    pub fn into_parent(
        self,
    ) -> LeavePath<AppPath<'a>, ComposeUp<NavPath<'a>, ComposeUp<LayerPath<'a>, Terminal>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp::new(ComposeUp::new(Terminal)),
        }
    }
}

impl<'a> LeavePath<AppPath<'a>, ComposeUp<NavPath<'a>, ComposeUp<LayerPath<'a>, Terminal>>> {
    /// bare root: Up(Up(app))
    pub fn complete(self) -> NavAscent<'a> {
        Doll::Up(Doll::Up(self.focus))
    }
}
```

## Full impls: leave started at `LayerPath` (parent is root)

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
            focus: self.focus.into_parent(),
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

## Walk on `PathMut`

```text
nav: PathMut<Nav, PathMut<Layer, &mut App>>

leave_at_nav(nav)
  focus = nav
  complete → Here(nav)

.into_parent()                    // PathMut::into_parent
  focus = layer : PathMut<Layer, &mut App>
  complete → Up(Here(layer))

.into_parent()                    // PathMut::into_parent
  focus = app : &mut App
  complete → Up(Up(app))
```

## Root

| | |
| --- | --- |
| Root path type | `&mut App` = `Place::Path` for root — **not** a `PathMut` |
| In `NavAscent` | only as bare innermost type of last `Up` |
| `into_parent` on `&mut App` | does not exist |
| Detect peel-to-root | `PathMut<N, Parent>` with `Parent = &mut T` installs `Terminal` |
| Leave **from** root | root dispatch does not `leave_at(&mut app)`; only unpacks child’s doll |

No `!` / `CNil` required. Optional later: uniform `Doll<AppPath, CNil>` if every layer must be an Either; not the default.

## Coproduct / inject (what this is, what it is not)

`Doll` is a two-variant coproduct (`Inl`/`Inr` = `Here`/`Up`), nestable like frunk’s `Coproduct` / `Result` generalized past two types.

**Construction is by peel history, not by type search.**

```text
// frunk-style inject: count Inrs by searching the type list for T
// peel-style: one Up per PathMut::into_parent call on this LeavePath
```

`finish` composition (`lift ∘ Here`) is the same *idea* as `inject`, but the number of `Up`s comes from the call chain stored in `ComposeUp`, not from `CoprodInjector<T, Index>` inference.

`PathMut<Node, Parent>` is indexed by **node and parent**, so two different spine positions are different types even if `Node` matched. Peel-history stays well-defined if laserbeam later allowed recursive `PathMut<Node, PathMut<Node, _>>`; type-search inject would go ambiguous there.

**No frunk dependency.** Steal ideas only if useful later: empty-enum terminal (`CNil`) for uniform layers; fold-over-handlers for parent consumers in invalidation. Newtyping frunk to rename `Inl`/`Inr` → `Here`/`Up` kills `inject`/`embed` — not worth the crate for a two-variant enum we own.

Runtime zippers (`Option` parent, erased path) are **not** this: they drop “how far up” from the type.

`Id` / `ComposeUp` is the composed constructor chain (difference-list / Cayley representation of the finish function): store the pipeline of `Up`s, apply at `complete`, not a runtime `Vec` of ops.

## Tests (deliverable)

Build real trees with `PathMut::from_fn` (same as bind tests: App / Layer / Nav).

```rust
#[test]
fn nav_complete_at_nav() {
    // … from_fn …
    assert!(matches!(leave_at_nav(nav).complete(), Doll::Here(_)));
}

#[test]
fn nav_one_peel_layer() {
    assert!(matches!(
        leave_at_nav(nav).into_parent().complete(),
        Doll::Up(Doll::Here(_))
    ));
}

#[test]
fn nav_two_peels_app() {
    assert!(matches!(
        leave_at_nav(nav).into_parent().into_parent().complete(),
        Doll::Up(Doll::Up(_))
    ));
}

#[test]
fn layer_peel_to_app() {
    assert!(matches!(
        leave_at_layer(layer).into_parent().complete(),
        Doll::Up(_)
    ));
}

#[test]
fn nav_ascent_type() {
    let out: NavAscent<'_> = leave_at_nav(nav).into_parent().into_parent().complete();
    let _ = out;
}
```

## Ordered changes

### 1 — `Doll`, `Id`, `ComposeUp`, `Terminal`, `LeavePath` in bind (or small module)

### 2 — `leave_at_nav` / `leave_at_layer` style APIs for fixtures; tests above

### 3 — Macro or derive helper: given `PathMut` nest depth, emit the `complete`/`into_parent` chain for that origin path type

### 4 — invalidation leave/kill calls this on `Place::Path`

## Rules

1. Focus is `PathMut` or root `&mut T` only. Peel is `PathMut::into_parent`.
2. Origin nest = `Doll<ThisPathMut, Doll<ParentPath, … AppPath>>` with root bare.
3. Start `f = id`; each peel composes one `Up`; `complete` runs `f` on `Here(focus)` until root uses `Terminal`.
4. Public `Here`/`Up`. Construction by peel count in type-state, not inject-by-type.
5. No second path ADT. No frunk required.
