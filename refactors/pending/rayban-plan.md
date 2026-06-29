# Rayban — design and implementation

Mutable counterpart to isograph's `resolve_position`. You put `#[derive(Rayban)]` on your state types and call `<Root as Resolve>::resolve(&mut root)`; it returns a typed path to the single active leaf, which mutates the leaf and walks up to its ancestors, one live `&mut` at a time. No `Rc`, no `RefCell`, no `unsafe`.

Two crates in the freddie workspace: `rayban` (the runtime, plus the re-exported derive) and `rayban_macro` (the derive). They depend on nothing else in the workspace. Both are implemented; the worked example below is `crates/rayban/tests/derived.rs`, and the hand-written shape it generates is `crates/rayban/tests/tree.rs`.

## Surface

```rust
use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = MediaResolved)]
enum MediaType { Album(Album), Song(Song) }

let resolved = <MediaType as Resolve>::resolve(&mut media); // -> MediaResolved
```

`resolve` takes the node's path by value (the root's path is `&mut Self`), so it is not a `self` method and there is no `media.resolve()` form.

## Attributes

`#[derive(Rayban)]` reads exactly one role attribute, so a contradictory combination cannot be written:

- `#[rayban_root(resolved = R)]` on the root node.
- `#[rayban(path = P, resolved = R)]` on a non-root node, where `P` is the node's own path type and `R` is the shared resolved enum.
- `#[rayban_path(node = N)]` on a multi-parent node's path enum, where `N` is the node that enum is the path for.

`#[resolve_into]` marks the one struct field a non-leaf struct descends into.

## Runtime (`rayban`)

```rust
enum Proj<Node, Parent> {                                  // private
    Bare(fn(&mut Parent) -> &mut Node),                    // what the derive emits; captures nothing
    Dyn(Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>), // hand-written, may capture (e.g. an index)
}

pub struct Path<Node, Parent> {
    parent: Parent,                                        // private
    projection: Proj<Node, Parent>,
}

impl<Node, Parent> Path<Node, Parent> {
    pub const fn from_fn(parent: Parent, projection: fn(&mut Parent) -> &mut Node) -> Self;
    pub fn from_box(parent: Parent, projection: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>) -> Self;
    pub fn get_mut(&mut self) -> &mut Node;                // re-derives the focused node
    pub const fn parent(&self) -> &Parent;                 // shared read up, without consuming
    pub fn into_parent(self) -> Parent;                    // consumes, moving one level up
}

pub trait Resolve {
    type Path<'a> where Self: 'a;                          // root: &'a mut Self
    type Resolved<'a> where Self: 'a;                      // the shared enum of all leaves
    fn resolve<'a>(path: Self::Path<'a>) -> Self::Resolved<'a> where Self: 'a;
}
```

`parent` is private, so up is `into_parent` (consuming) or `parent` (shared read). `get_mut(&mut self)` borrows the whole cursor. Those two facts make a stale or aliasing reference a compile error rather than a runtime bug; the `compile_fail` doctests on `Path` show both, and `resolve` taking the path by value (not `&mut self`) is what lets the `&mut` move down the tree with only one borrow live at a time.

## The route-enum design, and why

A node's path is `Path<Node, ParentPath>`. The projection re-derives the focused node from the parent, because we cannot store a `&mut` chain up to the root without aliasing.

For a node with one parent the path is a plain alias, e.g. `type AlbumPath<'a> = Path<Album, &'a mut MediaType>`. For a node with several parents the path is a local enum, one variant per parent:

```rust
#[derive(Rayban)]
#[rayban_path(node = Title)]
enum TitlePath<'a> {
    Album(Path<Title, AlbumPath<'a>>),
    Song(Path<Title, SongPath<'a>>),
}
```

This enum form is forced by the orphan rule. The earlier plan used isograph's single-path-type shape, `type TitlePath = Path<Title, TitleParent>`, and built it with a `From` impl. That cannot work once `Path` lives in the `rayban` crate: `impl From<..> for Path<Title, ..>` is an impl of a foreign trait for a foreign type (only the type arguments are local), which the orphan rule rejects (E0117). Making the path a local enum makes its `From` impls legal, since the target type is local. Single-parent nodes never hit this, because nothing converts into their path. `crates/rayban/tests/tree.rs` is the cross-crate proof.

## Model rules

- One value owns the tree; cursors borrow it; one live `&mut` at a time.
- A struct is a leaf unless it has one `#[resolve_into]` field, in which case `resolve` descends into it. At most one `#[resolve_into]` per struct.
- An enum's variants must all be single-field tuple variants `Foo(Bar)` whose payload is a node; `resolve` descends into the active variant's payload. Any other variant shape is a compile error.
- The `Resolved` enum has a variant per leaf, named after the leaf type. The derive constructs `Resolved::<NodeName>(path)` at a leaf.

## What you write vs what the derive generates

You declare the types, because your own code references them: the state types, the path types (a `Path<Node, ParentPath>` alias for one parent, a `#[rayban_path]` enum for several), and the `Resolved` enum.

The derive emits impls only:

- For a node (`#[rayban_root]` / `#[rayban]`): `impl Resolve`. Its `resolve` body, for a struct, is `<Child as Resolve>::resolve(Path::from_fn(path, edge).into())`, where `edge` is the `#[resolve_into]` field projection; for a leaf it is `Resolved::<Self>(path)`; for an enum it matches the active variant and descends. The root's `Path` is `&mut Self` and its descent dereferences it; a non-root node re-derives through `path.get_mut()`.
- For a path enum (`#[rayban_path]`): a `From<variant payload>` per variant (the trivial wrap), and a `get_mut` that dispatches to the active variant.

The descent is uniformly `from_fn(path, edge).into()`. For a single-parent child the `.into()` is the identity; for a multi-parent child it is the generated wrap into the path enum. The derive cannot see a child's parent count, so it always emits `.into()` and lets the type checker pick; the generated impl carries `#[allow(clippy::useless_conversion)]` for the identity case.

## Worked example

```rust
use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = MediaResolved)]
enum MediaType { Album(Album), Song(Song) }

#[derive(Rayban)]
#[rayban(path = AlbumPath, resolved = MediaResolved)]
struct Album { #[resolve_into] title: Title }

#[derive(Rayban)]
#[rayban(path = SongPath, resolved = MediaResolved)]
struct Song { #[resolve_into] title: Title }

#[derive(Rayban)]
#[rayban(path = TitlePath, resolved = MediaResolved)]
struct Title { #[resolve_into] credit: Credit }

#[derive(Rayban)]
#[rayban(path = CreditPath, resolved = MediaResolved)]
struct Credit { name: String }

type AlbumPath<'a> = Path<Album, &'a mut MediaType>;
type SongPath<'a> = Path<Song, &'a mut MediaType>;

#[derive(Rayban)]
#[rayban_path(node = Title)]
enum TitlePath<'a> {
    Album(Path<Title, AlbumPath<'a>>),
    Song(Path<Title, SongPath<'a>>),
}

type CreditPath<'a> = Path<Credit, TitlePath<'a>>;

enum MediaResolved<'a> { Credit(CreditPath<'a>) }
```

Resolving lands on `MediaResolved::Credit`; `get_mut` mutates the name; `into_parent` returns `TitlePath`, which you match to learn the parent and walk further up.

## Tests

- `Path` unit tests, and `compile_fail` doctests for the aliasing guarantee (E0505), the staleness guarantee (use after `into_parent`), and the private parent field.
- `tests/tree.rs`: the example with hand-written impls, in a crate separate from `rayban` (the orphan-rule proof). Resolves through each branch, mutates the leaf, walks up to mutate an ancestor, reads an ancestor through `parent`.
- `tests/derived.rs`: the same tree via `#[derive(Rayban)]`, plus a resolve / mutate / re-resolve round trip.
- `tests/enum_node.rs`: a non-root enum node, exercising the `matches!` descent.
- `tests/compile_fail.rs` (trybuild): the derive's own error messages for a missing role attribute, two role attributes, two `#[resolve_into]`, and a non-`Foo(Bar)` enum variant.

## Lints and CI

`unsafe_code = "forbid"` and clippy `all`/`pedantic`/`nursery`/`cargo` denied at the workspace level, with narrow allows where a denial is wrong (`multiple_crate_versions` and `cargo_common_metadata`, both driven by the dependency tree; `mut_mut`, because the root cursor is a `&mut Root` and a projection over it is `&mut &mut Root`). Generated impls are `#[automatically_derived]` and carry the one allow they need.

`.github/workflows/ci.yml` runs `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo test --all --all-features`, on a pinned 1.96.0 toolchain so clippy and trybuild match locally. A push to `master` publishes `0.0.1-main-<7-char sha>` of every crate; a `v*` tag publishes that version. The version step rewrites both the workspace version and the path-dependency version specifiers, so the published `rayban` depends on the matching `rayban_macro`.

## Deferred (post-v1)

- `#[resolve_into]` over a `Vec` or collection, descent driven by a stored index. The runtime already takes a hand-written indexed box (a `Proj::Dyn`); macro support comes after the struct and enum cases.
- `Option` and `Result`, both as a node's own type and as a `#[resolve_into]` field. As a node they are ordinary enums (`Some`/`Ok` descend, `None` is terminal, `Err` is terminal or descends into a node), with the obstacle that the orphan rule blocks deriving on `std`'s types, so the macro would special-case them. As a `#[resolve_into]` field the edge becomes conditional and `Resolved` gains a terminal variant for the empty or error case.
- Generic node types. The derive currently rejects them.
