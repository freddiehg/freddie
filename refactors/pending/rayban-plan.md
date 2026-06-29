# Rayban — implementation plan

Mutable counterpart to isograph's `resolve_position`. You write `#[derive(Rayban)]` on your state types and call `value.resolve()`; it returns a typed cursor to the single active leaf, which reads/mutates the leaf and walks up to ancestors, one live `&mut` at a time. No `Rc`, `RefCell`, or `unsafe`. Two crates in the freddie workspace, depending on nothing else in it.

The runtime, the path types, `get_mut`/`into_parent`, multi-parent route enums, and the full descent are compile-validated as standalone prototypes; the macro emits exactly that code.

## Surface

```rust
#[derive(Rayban)]
#[rayban_root(resolved = MediaResolved)]
enum MediaType { Album(Album), Song(Song) }

let resolved = <MediaType as Resolve>::resolve(&mut media);   // -> MediaResolved, the active leaf's path
```

## Names

- Crate: `rayban` (runtime + re-exported derive); `rayban_macro` (proc-macro).
- Derive: `#[derive(Rayban)]`, with the attribute saying which role: `#[rayban_root(resolved = ..)]` on the root, `#[rayban(parent_type = .., resolved = ..)]` on a non-root node, `#[rayban_parent(child = ..)]` on a multi-parent node's parent enum. Separate attributes rather than flags, so a contradictory combination like `rayban(root, parent_type = ..)` isn't representable. A node attribute produces an `impl Resolve` (the root's differs: its `Path` is `&mut Self`, its `resolve` does the top-level match-descent); `rayban_parent` produces the parent enum's projection and `From` wrappers.
- Trait: `Resolve`, implemented at every layer (like isograph's `ResolvePosition`). `resolve` is a trait fn, not inherent, so it can't collide with a type's own methods.
- `Path<Node, Parent>`, with `get_mut`, `parent` (shared ref to the parent), and `into_parent`. There is no `Root` type; the root is just `&mut T`.
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

The macro also implements, on each node that has a `#[resolve_into] f: Child`, a small projection trait:

```rust
pub trait Projection<Child> {
    fn child_mut(&mut self) -> &mut Child;
}
```

`Resolve` is implemented at every layer (like isograph's `ResolvePosition`). `resolve` takes the node's path by value, not `&mut self`, which is what avoids the E0499 the top-down `&mut` walk hits: the `&mut` is moved into the path once, and each layer re-derives its node through `get_mut`. The root's `type Path<'a> = &'a mut Self`, so the entry is `Self::resolve(&mut value)`.

A descending node names no child parent type. It builds a `Path` over its own path and `.into()`s it into the child's path type (inferred from `<Child as Resolve>::Path`). The wrapping `From` carries the variant, and it can be a trivial wrap because the box already lives in the path being wrapped, so there is no pattern through an associated type (unstable, rust#86935) and no reach into `Path`'s private fields. A multi-parent node's projection dispatches through `Projection`, so the field `#[resolve_into]` named never has to be visible to the parent enum's derive. `get_mut`/`parent`/`into_parent` stay on `Path`, not the trait.

## Model rules

- One value owns the tree; cursors borrow it; one live `&mut` at a time.
- Enum: every variant must be a single-field tuple variant `Foo(Bar)` whose payload `Bar` is a Rayban node. `resolve` descends into the active variant's `Bar` (a variant projection, matching the current variant). Any other variant shape (unit, struct-like, multi-field, or a non-node payload) is a compile error.
- Struct: a leaf, unless it has one `#[resolve_into]` field, then descend into it (a field projection, always succeeds). At most one `#[resolve_into]` per struct.
- Plain fields are state. Shared state lives on a parent struct, reached from below via the cursor. Behavioral flags are fields, not variants.
- Exactly one active leaf, picked by a discriminator (enum variant, or a state tag/index over stored children). To keep inactive siblings across a toggle, store them as fields plus a tag, not as an enum.

## The running example

`Album` and `Song` each have a `Title`. "A Kind of Magic" is both a song and an album, so `Title` has two parents.

```rust
#[derive(Rayban)]
#[rayban_root(resolved = MediaResolved)]
enum MediaType { Album(Album), Song(Song) }

#[derive(Rayban)]
#[rayban(parent_type = AlbumParent, resolved = MediaResolved)]
struct Album { #[resolve_into] title: Title, year: u32 }

#[derive(Rayban)]
#[rayban(parent_type = SongParent, resolved = MediaResolved)]
struct Song { #[resolve_into] title: Title }

#[derive(Rayban)]
#[rayban(parent_type = TitleParent, resolved = MediaResolved)]
struct Title { text: String }
```

`parent_type =` names this node's parent type (also exactly what `into_parent` returns); the macro derives the node's path as `type Path<'a> = Path<Self, ParentType<'a>>`. `resolved =` names the `Resolved` enum. No child parents are listed — a descending node never needs them, it just `.into()`s its own path. The macro reads these and emits impls only — it never creates a type.

## What you write vs what the macro generates

You declare every type, because other code references them:

- the state types;
- each node's parent type (named by `parent_type =`, and exactly what `into_parent` returns): a single-parent node's is its parent's path (e.g. `type AlbumParent<'a> = &'a mut MediaType`); a multi-parent node's is an enum (`TitleParent`);
- the `Resolved` enum.

A multi-parent node's parent enum carries its own `#[derive(Rayban)]` with a `#[rayban_parent(child = Title)]` attribute, so you write the enum but not its impls. You hand-write nothing for either case.

The macro generates impls only:

- For a node (`#[rayban]` / `#[rayban_root]`): `impl Resolve` with `type Path<'a>` (= `Path<Self, ParentType<'a>>` from `parent_type`; the root's is `&'a mut Self`), `type Resolved` (= your `resolved`), and `resolve` (the descent is `<Child as Resolve>::resolve(self_path.into())`, or at a leaf `Resolved::Self(path)`). It also gets a `Projection` impl per edge: a struct with `#[resolve_into] f: Child` gets `impl Projection<Child>` returning `&mut self.f`; an enum gets `impl Projection<Bar>` per `Foo(Bar)` variant, a match returning that variant's payload (other arms dead by the consume invariant).
- For a parent enum (`#[rayban_parent(child = C)]`): the projection `&mut Self -> &mut C` (a `match` whose arms call `Projection::<C>::child_mut` on each variant, so no field name appears here), plus the `From<ParentPath> for Path<C, Self>` wrappers that `.into()` selects.

So the field access lives only where `#[resolve_into]` declares it; the parent enum's derive reaches it through `Projection` and never names a field. Single-parent nodes don't have a parent enum, so the edge `From` and its box are emitted directly by the node's derive.

## The code (you write the types; the macro writes the impls), validated

```rust
// You write these types.

// Single-parent nodes: parent_type is the parent's own path.
type AlbumParent<'a> = &'a mut MediaType;
type SongParent<'a> = &'a mut MediaType;
type AlbumPath<'a> = Path<Album, AlbumParent<'a>>; // = the macro's `type Path` for Album
type SongPath<'a> = Path<Song, SongParent<'a>>;

// Multi-parent node: parent_type is an enum, and it carries its own derive.
#[derive(Rayban)]
#[rayban_parent(child = Title)]
enum TitleParent<'a> {
    Album(AlbumPath<'a>),
    Song(SongPath<'a>),
}
type TitlePath<'a> = Path<Title, TitleParent<'a>>;

enum MediaResolved<'a> {
    Title(TitlePath<'a>),
}

// The macro generates everything below.

// From each parent node's `#[resolve_into]` (the field is known there):
impl Projection<Title> for Album {
    fn child_mut(&mut self) -> &mut Title {
        &mut self.title
    }
}

impl Projection<Title> for Song {
    fn child_mut(&mut self) -> &mut Title {
        &mut self.title
    }
}

// From the parent enum's derive. No field name appears here; it delegates to child_mut.
fn title_from_parent<'p, 'a>(tp: &'p mut TitleParent<'a>) -> &'p mut Title {
    match tp {
        TitleParent::Album(ap) => ap.get_mut().child_mut(),
        TitleParent::Song(sp) => sp.get_mut().child_mut(),
    }
}

impl<'a> From<AlbumPath<'a>> for TitlePath<'a> {
    fn from(ap: AlbumPath<'a>) -> Self {
        Path::from_fn(TitleParent::Album(ap), title_from_parent)
    }
}

impl<'a> From<SongPath<'a>> for TitlePath<'a> {
    fn from(sp: SongPath<'a>) -> Self {
        Path::from_fn(TitleParent::Song(sp), title_from_parent)
    }
}

// The Resolve impls. No descent names a parent type.
impl Resolve for MediaType {
    type Path<'a> = &'a mut MediaType;
    type Resolved<'a> = MediaResolved<'a>;

    fn resolve<'a>(media: &'a mut MediaType) -> MediaResolved<'a>
    where
        Self: 'a,
    {
        match media {
            MediaType::Album(_) => <Album as Resolve>::resolve(Path::from_fn(media, |o| {
                let MediaType::Album(a) = &mut **o else { unreachable!() };
                a
            })),
            MediaType::Song(_) => <Song as Resolve>::resolve(Path::from_fn(media, |o| {
                let MediaType::Song(s) = &mut **o else { unreachable!() };
                s
            })),
        }
    }
}

impl Resolve for Album {
    type Path<'a> = AlbumPath<'a>;
    type Resolved<'a> = MediaResolved<'a>;

    fn resolve<'a>(p: AlbumPath<'a>) -> MediaResolved<'a>
    where
        Self: 'a,
    {
        <Title as Resolve>::resolve(p.into())
    }
}

impl Resolve for Song {
    type Path<'a> = SongPath<'a>;
    type Resolved<'a> = MediaResolved<'a>;

    fn resolve<'a>(p: SongPath<'a>) -> MediaResolved<'a>
    where
        Self: 'a,
    {
        <Title as Resolve>::resolve(p.into())
    }
}

impl Resolve for Title {
    type Path<'a> = TitlePath<'a>;
    type Resolved<'a> = MediaResolved<'a>;

    fn resolve<'a>(p: TitlePath<'a>) -> MediaResolved<'a>
    where
        Self: 'a,
    {
        MediaResolved::Title(p)
    }
}
```

The descent names no parent type. Each step builds a `Path` over its own incoming path and either `.into()`s it to the child's path type (inferred from `<Child as Resolve>::Path`, the `From` carrying the variant) or, at a leaf, returns `Resolved::Self(path)`. The `&mut` is moved into the path once and each layer re-derives its node through `get_mut`, which is what avoids the E0499 the naive top-down `&mut` walk hits. The root's box derefs the bare `&mut` (`&mut **o`); per-edge boxes capture nothing, so they're bare `fn`s. Nothing is hand-written: a multi-parent node's projection and `From`s come from the `#[rayban_parent]` derive, which reaches each parent's edge through the derived `Projection`.

## Macro deps (`rayban_macro`)

`syn`, `quote`, `proc-macro2`, `deluxe` (parse `#[rayban(...)]` / `#[rayban_root(...)]` / `#[rayban_parent(...)]`), `convert_case` (ident casing). One `#[proc_macro_derive(Rayban, attributes(rayban, rayban_root, rayban_parent, resolve_into))]`, dispatching on which attribute is present (`rayban_root` = root, `rayban` = non-root node, `rayban_parent` = parent enum) and on enum vs struct, modeled on isograph's `resolve_position_macros`.

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

`trybuild` compile-fail, each a macro error with a clear message: the two guarantees; two `#[resolve_into]` on one struct; a `#[resolve_into]` field whose type is not a node; an enum variant that is not `Foo(Bar)` with `Bar` a node (unit, struct-like, multi-field, or a non-node payload); and the attribute states the design rules out:

- `#[rayban]` and `#[rayban_root]` on the same type,
- two `#[rayban]` attributes on one type,
- two `#[rayban_root]` attributes on one type,
- a `#[derive(Rayban)]` type with neither attribute.

## Build order

1. Done: standalone prototypes (runtime `Path` with `Proj` fn/box, the isograph-shape path aliases, `.into()` descent, multi-parent projection) compile and run.
2. `rayban` runtime from the code above; tests against hand-written path types and hand-written `Resolve` impls.
3. `rayban_macro`: parse with `deluxe`; emit the `Resolve` impls (assoc types plus `resolve` with the `.into()` descent and per-edge boxes) to match the prototype; rerun the tests with the hand-written impls replaced by `#[derive(Rayban)]`.

## Deferred (post-v1)

- `#[resolve_into]` over a `Vec`/collection, descent driven by a stored index or discriminator. The runtime already takes a hand-written indexed box (a `Proj::Dyn`); macro support comes after the enum and struct cases.
- `Option` and `Result`, both as a node's own type and as a `#[resolve_into]` field. Sketch below; not in v1.

### Option and Result

As a node's own type (a node whose type is `Option<T>` or `Result<T, E>`): these are ordinary enums, so the normal variant descent fits. `Some(T)` and `Ok(T)` descend into the payload, `None` is terminal, and `Err(E)` is terminal or descends into `E` when `E` is itself a node. The obstacle is the orphan rule, since `#[derive(Rayban)]` can't sit on `std`'s `Option`/`Result`. The macro special-cases the two types and emits their `Resolve` impls directly; a local newtype is the fallback but reads worse for callers.

As a `#[resolve_into]` field, the edge becomes conditional. `#[resolve_into] child: Option<Child>` descends into `Child` when the field is `Some`, and when it is `None` the node itself is the active leaf, so `resolve` branches on the field and `Resolved` gains a variant for the node stopping there. The projection box assumes `Some`; its `None` arm is dead by the consume invariant. `#[resolve_into] child: Result<Child, E>` descends into `Child` on `Ok`, and on `Err` it either stops with the error as the leaf or descends into `E`, so `Resolved` gains a variant carrying the error. The cursor mechanics are unchanged in both: one `&mut` re-derived per level, the box sitting behind a `Some` or `Ok` pattern.

## CI / publish

Implemented in `.github/workflows/ci.yml`: `cargo-fmt`, `cargo-clippy`, `cargo-test`, `all-checks-passed`, then `main-release` (push to `master` → `0.0.1-main-<7-char sha>`) and `versioned-release` (`v*` tag).
