# laserbeam: ascend → ancestor

Done. Standalone. Prefactor for `path-peel-complete.md` (which uses `path_nest!`).

Pure rename, no behavior change. The up-walk family speaks the nouns the one-level walk already uses: one level is `parent` / `into_parent`, many levels become `ancestor` / `into_ancestor`. The `Mut` suffix dies because `ascend_mut` consumes, while everywhere else in laserbeam `mut` means borrow (`get_mut`) and consuming is spelled `into_` (`into_parent`).

## Renames

```text
trait Ascend<Target>      → trait HasAncestor<Target>
  fn ascend(&self)        →   fn ancestor(&self) -> &Target
trait AscendMut<Target>   → trait IntoAncestor<Target>: HasAncestor<Target>
  fn ascend_mut(self)     →   fn into_ancestor(self) -> Target

PathMut::ascend_to        → PathMut::ancestor       (inherent sugar)
PathMut::ascend_to_mut    → PathMut::into_ancestor  (inherent sugar)

macro ascend_nest!        → path_nest!
macro ascend_up!          → into_parent_chain!
macro ascend_up_ref!      → parent_chain!
macro ascend_impls!       → ancestor_impls!

mod ascend_tests          → mod ancestor_tests
```

## laserbeam (before / after)

Traits:

```rust
// before
pub trait Ascend<Target> {
    fn ascend(&self) -> &Target;
}
pub trait AscendMut<Target>: Ascend<Target> {
    fn ascend_mut(self) -> Target;
}

// after
pub trait HasAncestor<Target> {
    fn ancestor(&self) -> &Target;
}
pub trait IntoAncestor<Target>: HasAncestor<Target> {
    fn into_ancestor(self) -> Target;
}
```

Identity impls keep their shape (`impl<T> HasAncestor<T> for T`, `impl<T> IntoAncestor<T> for T`).

Sugar: the inherent methods take the trait methods' own names. Inherent methods win method resolution, so `path.ancestor::<T>()` and `path.ancestor()` both land on the sugar, which delegates; the separate `_to` names existed only because the old sugar could not shadow. (Same-name shadowing compile-checked.)

```rust
// before
pub fn ascend_to<Target>(&self) -> &Target where Self: Ascend<Target> { Ascend::ascend(self) }
pub fn ascend_to_mut<Target>(self) -> Target where Self: AscendMut<Target> { AscendMut::ascend_mut(self) }

// after
pub fn ancestor<Target>(&self) -> &Target
where
    Self: HasAncestor<Target>,
{
    HasAncestor::ancestor(self)
}

pub fn into_ancestor<Target>(self) -> Target
where
    Self: IntoAncestor<Target>,
{
    IntoAncestor::into_ancestor(self)
}
```

Macros:

```rust
// before
macro_rules! ascend_nest {
    ($t:ident) => { $t };
    ($t:ident, $head:ident $(, $rest:ident)*) => {
        PathMut<$head, ascend_nest!($t $(, $rest)*)>
    };
}

// after — renamed, and the terminal widens to any type
macro_rules! path_nest {
    ($t:ty) => { $t };
    ($t:ty, $head:ident $(, $rest:ident)*) => {
        PathMut<$head, path_nest!($t $(, $rest)*)>
    };
}
```

`into_parent_chain!` / `parent_chain!` / `ancestor_impls!` are token-for-token the old bodies with the new names; `ancestor_impls!` emits `HasAncestor`/`IntoAncestor` impls calling `parent_chain!`/`into_parent_chain!`. The depth-12 invocation is unchanged. Doc comments on `PathMut` and the traits rename their references (`ascend_to` → `ancestor`, "Ascend/AscendMut" → "HasAncestor/IntoAncestor").

## mercury (before / after)

Mechanical; the only call forms in the tree are bare `.ascend()` / `.ascend_mut()` with trait bounds (no `ascend_to` call sites exist outside laserbeam).

```rust
// before (state/app.rs)
use laserbeam::Ascend;
fn app_data<'a, P: Ascend<MercuryPath<'a>>>(path: &P) -> Option<AppData> {
    let root = path.ascend();
    …
}

// after
use laserbeam::HasAncestor;
fn app_data<'a, P: HasAncestor<MercuryPath<'a>>>(path: &P) -> Option<AppData> {
    let root = path.ancestor();
    …
}
```

```rust
// before (handlers/home.rs and friends)
use laserbeam::AscendMut;
pub(crate) fn to_home<'a, E, P: AscendMut<MercuryPath<'a>>>(…) -> … {
    go_home(node.parent.ascend_mut())
}

// after
use laserbeam::IntoAncestor;
pub(crate) fn to_home<'a, E, P: IntoAncestor<MercuryPath<'a>>>(…) -> … {
    go_home(node.parent.into_ancestor())
}
```

Files: `state/app.rs`, `state/site.rs` (`Ascend` → `HasAncestor`, `.ascend()` → `.ancestor()`); `handlers/home.rs`, `quit.rs`, `overlay.rs`, `mod.rs`, `nav.rs`, `resize.rs`, `app.rs` (`AscendMut` → `IntoAncestor`, `.ascend_mut()` → `.into_ancestor()`); prose in `handlers/home.rs` ("ascends to the root" → "reaches the root ancestor") and `handlers/mod.rs` ("has already ascended" → "already holds the root").

## AGENTS.md

The handler-bounds bullet and the laserbeam-vs-bind section rename their mentions:

```text
`P: AscendMut<MercuryPath<'a>>`  → `P: IntoAncestor<MercuryPath<'a>>`
`P: Ascend<MercuryPath<'a>>`     → `P: HasAncestor<MercuryPath<'a>>`
`Ascend`/`AscendMut`             → `HasAncestor`/`IntoAncestor`
"walk up with `into_parent` / `Ascend` / `AscendMut`"
                                 → "walk up with `into_parent` / `HasAncestor` / `IntoAncestor`"
```

## Ordered changes

One shippable change; the workspace renames atomically or it does not compile.

### 1 — laserbeam traits/methods/sugar/macros/tests/docs; mercury bounds and call sites; AGENTS.md wording

## Rules

1. Pure rename plus the `path_nest!` terminal widening (`ident` → `ty`); no signatures change shape, no impls are added or removed.
2. One level: `parent` / `into_parent`. Many levels: `ancestor` / `into_ancestor`. `mut` means borrow, `into_` means consume, everywhere.
3. The verb "ascend" no longer appears in laserbeam or its consumers.
