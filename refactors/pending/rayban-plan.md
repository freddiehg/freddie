# Rayban — implementation plan

Mutable counterpart to isograph's `resolve_position`. You write `#[derive(Rayban)]` on your state types and call `value.resolve()`; it returns a typed cursor to the single active leaf, which reads/mutates the leaf and walks up to ancestors, one live `&mut` at a time. No `Rc`, `RefCell`, or `unsafe`. Two crates in the freddie workspace, depending on nothing else in it.

The runtime, the path types, `get_mut`/`into_parent`, multi-parent route enums, and the full descent are compile-validated as standalone prototypes; the macro emits exactly that code.

## Surface

```rust
#[derive(Rayban)]
enum MediaType { Album(Album), Song(Song) }

let resolved = <MediaType as Resolve>::resolve(&mut media);   // -> MediaResolved, the active leaf's path
```

## Names

- Crate: `rayban` (runtime + re-exported derive); `rayban_macro` (proc-macro).
- Derive: `#[derive(Rayban)]` implements the `Resolve` trait.
- Trait: `Resolve`, implemented at every layer (like isograph's `ResolvePosition`). `resolve` is a trait fn, not inherent, so it can't collide with a type's own methods.
- Cursor struct: `Path<Node, Parent>`, with `get_mut`, `parent` (shared ref to the parent), and `into_parent`. There is no `Root` type; the root is just `&mut T`.
- Entry: `<MediaType as Resolve>::resolve(&mut media)`. `resolve` takes the node's path by value (the root's path is `&mut Self`), so it isn't `&self` and there's no `value.resolve()` method form.
- One term: a node's cursor is its "Path" everywhere (no "Route").

## Lints (both crates)

```rust
#![forbid(unsafe_code)]
```
plus workspace lints set as strict as practical:
```toml
[workspace.lints.rust]
unsafe_code = "forbid"
[workspace.lints.clippy]
all = "deny"
pedantic = "deny"
nursery = "deny"
cargo = "deny"
```
Specific lints get `#[allow(...)]` with a reason only where a denial is wrong, never blanket-relaxed.

## Runtime (`rayban`)

```rust
pub enum Proj<Node, Parent> {
    Bare(fn(&mut Parent) -> &mut Node),                         // derived projections capture nothing
    Dyn(Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>),   // hand-written projections may capture
}
pub struct Path<Node, Parent> {
    parent: Parent,                                             // private
    project: Proj<Node, Parent>,
}
impl<Node, Parent> Path<Node, Parent> {
    pub fn from_fn(parent: Parent, f: fn(&mut Parent) -> &mut Node) -> Self {
        Path { parent, project: Proj::Bare(f) }
    }
    pub fn from_box(parent: Parent, f: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>) -> Self {
        Path { parent, project: Proj::Dyn(f) }
    }
    pub fn get_mut(&mut self) -> &mut Node {
        match &self.project { Proj::Bare(f) => f(&mut self.parent), Proj::Dyn(f) => f(&mut self.parent) }
    }
    pub fn parent(&self) -> &Parent { &self.parent }
    pub fn into_parent(self) -> Parent { self.parent }
}
```

`parent` is private; up is `into_parent` (consuming) or `parent(&self)` (shared read). The projection is either a bare `fn` (what the derive emits — its match/field closures capture nothing, so no heap, no dispatch cost) or a `Box<dyn Fn>` (for a hand-written projection that closes over data the derive can't see, e.g. an externally supplied index). `from_fn`/`from_box` are public so the macro and hand-written impls construct cursors. The root is a bare `&mut T`: the first level's box derefs it (`&mut **o`), so no wrapper type exists.

## Trait

```rust
pub trait Resolve {
    type Path<'a> where Self: 'a;       // this node's path type; root: &'a mut Self
    type Resolved<'a> where Self: 'a;   // the shared enum of all leaves
    fn resolve<'a>(path: Self::Path<'a>) -> Self::Resolved<'a> where Self: 'a;
}
```

One trait, implemented at every layer (like isograph's `ResolvePosition`). `resolve` takes the node's path by value, not `&mut self` — that's what avoids the E0499 the top-down `&mut` walk hits: the `&mut` is moved into the path once, and each layer re-derives its node through `get_mut`. The root's `type Path<'a> = &'a mut Self`, so the entry is `Self::resolve(&mut value)`.

A descending node names no child parent type: it builds a `Path` over its own path and `.into()`s it into the child's path type (inferred from `<Child as Resolve>::Path`). The wrapping `From` carries the variant, and it can be a trivial wrap because the box already lives in the path being wrapped — no pattern through an associated type (which is unstable, rust#86935), and no reach into `Path`'s private fields. `get_mut`/`parent`/`into_parent` stay on `Path`, not the trait.

## Model rules

- One value owns the tree; cursors borrow it; one live `&mut` at a time.
- Enum: `resolve` descends the active variant (a variant projection — matches the current variant).
- Struct: a leaf, unless it has one `#[resolve_into]` field, then descend into it (a field projection — always succeeds). At most one `#[resolve_into]` per struct.
- Plain fields are state. Shared state lives on a parent struct, reached from below via the cursor. Behavioral flags are fields, not variants.
- Exactly one active leaf, picked by a discriminator (enum variant, or a state tag/index over stored children). To keep inactive siblings across a toggle, store them as fields plus a tag, not as an enum.

## The running example

`Album` and `Song` each have a `Title`. "A Kind of Magic" is both a song and an album, so `Title` has two parents.

```rust
#[derive(Rayban)]
#[rayban(root, resolved = MediaResolved)]
enum MediaType { Album(Album), Song(Song) }

#[derive(Rayban)]
#[rayban(path = AlbumPath, resolved = MediaResolved)]
struct Album { #[resolve_into] title: Title, year: u32 }

#[derive(Rayban)]
#[rayban(path = SongPath, resolved = MediaResolved)]
struct Song { #[resolve_into] title: Title }

#[derive(Rayban)]
#[rayban(path = TitlePath, resolved = MediaResolved)]
struct Title { text: String }
```

`path =` names this node's own path type (the alias you declared); `resolved =` names the `Resolved` enum you declared. No parents are listed — a descending node never needs its child's parents, it just `.into()`s its own path. The macro reads these and emits impls only — it never creates a type.

## What you write vs what the macro generates

You declare every type, because other code references them:

- the state types;
- a per-node path alias: `type AlbumPath<'a> = Path<Album, &'a mut MediaType>` for a single parent; for a multi-parent node, the parent enum (`TitleParent`) plus the alias over it (`type TitlePath<'a> = Path<Title, TitleParent<'a>>`);
- the `Resolved` enum.

For a multi-parent node you also write the projection (`title_from_parent`) and the `From` impls that `.into()` selects. A per-type derive can't write those — the projection needs each parent's edge (`Album.title`, `Song.title`), which only that parent's own item declares. Single-parent nodes need none of it: `ChildPath` is just `Path<Child, ParentPath>` and the derive writes the one box.

The macro generates impls only, per `#[derive(Rayban)]`:

- `impl Resolve`: `type Path` (= your `path`), `type Resolved` (= your `resolved`), `resolve`.
- the descent inside `resolve`: `<Child as Resolve>::resolve(self_path.into())`, or at a leaf `Resolved::Self(path)`.
- the per-edge projection box (a bare `fn`) for each `#[resolve_into]` field or active variant.

## The code (you write the types; the macro writes the impls), validated

```rust
// YOU write these types:
type AlbumPath<'a> = Path<Album, &'a mut MediaType>;   // single parent: alias straight to Path
type SongPath<'a>  = Path<Song, &'a mut MediaType>;
enum TitleParent<'a> { Album(AlbumPath<'a>), Song(SongPath<'a>) }   // multi-parent: the parent enum
type TitlePath<'a>  = Path<Title, TitleParent<'a>>;                 // ... and the path alias over it
enum MediaResolved<'a> { Title(TitlePath<'a>) }

// YOU write the multi-parent projection + the `.into()` wrappers (concrete enum -> stable match):
fn title_from_parent<'p, 'a>(tp: &'p mut TitleParent<'a>) -> &'p mut Title {
    match tp { TitleParent::Album(ap) => &mut ap.get_mut().title, TitleParent::Song(sp) => &mut sp.get_mut().title }
}
impl<'a> From<AlbumPath<'a>> for TitlePath<'a> { fn from(ap: AlbumPath<'a>) -> Self { Path::from_fn(TitleParent::Album(ap), title_from_parent) } }
impl<'a> From<SongPath<'a>>  for TitlePath<'a> { fn from(sp: SongPath<'a>)  -> Self { Path::from_fn(TitleParent::Song(sp), title_from_parent) } }

// THE MACRO generates the Resolve impls; the descent names no parent type:
impl Resolve for MediaType {
    type Path<'a> = &'a mut MediaType;
    type Resolved<'a> = MediaResolved<'a>;
    fn resolve<'a>(media: &'a mut MediaType) -> MediaResolved<'a> where Self: 'a {
        match media {
            MediaType::Album(_) => <Album as Resolve>::resolve(Path::from_fn(media,
                |o| { let MediaType::Album(a) = &mut **o else { unreachable!() }; a })),
            MediaType::Song(_)  => <Song as Resolve>::resolve(Path::from_fn(media,
                |o| { let MediaType::Song(s) = &mut **o else { unreachable!() }; s })),
        }
    }
}
impl Resolve for Album {
    type Path<'a> = AlbumPath<'a>;
    type Resolved<'a> = MediaResolved<'a>;
    fn resolve<'a>(p: AlbumPath<'a>) -> MediaResolved<'a> where Self: 'a { <Title as Resolve>::resolve(p.into()) }
}
impl Resolve for Song {
    type Path<'a> = SongPath<'a>;
    type Resolved<'a> = MediaResolved<'a>;
    fn resolve<'a>(p: SongPath<'a>) -> MediaResolved<'a> where Self: 'a { <Title as Resolve>::resolve(p.into()) }
}
impl Resolve for Title {
    type Path<'a> = TitlePath<'a>;
    type Resolved<'a> = MediaResolved<'a>;
    fn resolve<'a>(p: TitlePath<'a>) -> MediaResolved<'a> where Self: 'a { MediaResolved::Title(p) }
}
```

The descent names no parent type: each step builds a `Path` over its own incoming path and either `.into()`s it to the child's path type (inferred from `<Child as Resolve>::Path`, the `From` carrying the variant) or, at a leaf, returns `Resolved::Self(path)`. The `&mut` is moved into the path once and each layer re-derives its node through `get_mut`, which is what avoids the E0499 the naive top-down `&mut` walk hits. The root's box derefs the bare `&mut` (`&mut **o`); per-edge boxes capture nothing, so they're bare `fn`s. The one `&mut`-specific hand-written piece is a multi-parent node's `*_from_parent` projection plus its `From`s.

## Macro deps (`rayban_macro`)

`syn`, `quote`, `proc-macro2`, `deluxe` (parse `#[rayban(...)]`), `convert_case` (ident casing). One `#[proc_macro_derive(Rayban, attributes(rayban, resolve_into))]`, dispatching enum vs struct, modeled on isograph's `resolve_position_macros`.

## Usage

```rust
let mut media = MediaType::Album(Album { title: Title { text: "A Kind of Magic".into() }, year: 1986 });
match <MediaType as Resolve>::resolve(&mut media) {
    MediaResolved::Title(mut tp) => {
        tp.get_mut().text.push('!');           // mutate the leaf
        match tp.into_parent() {               // walk up to the album/song
            TitleParent::Album(mut a) => a.get_mut().year = 1986,
            TitleParent::Song(_) => {}
        }
    }
}
```

## Guarantees and tests

- Aliasing prevented statically: `get_mut(&mut self)` borrows the whole cursor, so leaf and ancestor `&mut` can't coexist. `trybuild` compile-fail holding both → E0499.
- Staleness prevented by encapsulation: `parent` private, `into_parent` consuming. `trybuild` compile-fail on external `.parent`; on `into_parent` then `get_mut` (use-after-move).

Behavior tests (`rayban/tests/`): resolve lands on the right leaf; `get_mut` mutates; `into_parent` walks up and mutates an ancestor; multi-parent resolve via each route; shared state on an intermediate read/mutated from a leaf; indexed leaf via a hand-written box; resolve/mutate/re-resolve round-trip.

`trybuild` compile-fail: the two guarantees, plus two `#[resolve_into]` on one struct → macro error.

## Build order

1. Done: standalone prototypes (runtime `Path` with `Proj` fn/box, the isograph-shape path aliases, `.into()` descent, multi-parent projection) compile and run.
2. `rayban` runtime from the code above; tests against hand-written path types and hand-written `Resolve` impls.
3. `rayban_macro`: parse with `deluxe`; emit the `Resolve` impls (assoc types plus `resolve` with the `.into()` descent and per-edge boxes) to match the prototype; rerun the tests with the hand-written impls replaced by `#[derive(Rayban)]`.

## Deferred

- `#[resolve_into]` over a `Vec`/collection (discriminator-driven). Runtime supports a hand-written indexed box; macro support comes after the enum/struct cases.
- A whole-tree derive to auto-generate a multi-parent node's `*_from_parent` projection and its `From`s (a per-type derive can't — it lacks each parent's edge). Until then those few are hand-written.

## CI / publish

Implemented in `.github/workflows/ci.yml`: `cargo-fmt`, `cargo-clippy`, `cargo-test`, `all-checks-passed`, then `main-release` (push to `master` → `0.0.1-main-<7-char sha>`) and `versioned-release` (`v*` tag).
