# rayban

Given a mutable reference to a deeply nested struct, rayban produces a `Path` struct capable of two things: providing a mutable reference to the active leaf node, and the ability to produce a `Path` that is one layer closer to the root.

In that sense, the `Path` acts as a "typed iterator" from leaf to root. However, unlike with traditional iterators, the types of items the "iterator" produces vary: in the following example, the "iterator" would first produce a `Single` and then a `MediaType`. And furthermore, these are mutually dependent: we know that the `MediaType` must follow a `Single`, and never vice versa.

## Example

```rust
use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = Resolved)]
enum MediaType {
    Album(Album),
    Single(Single),
}

#[derive(Rayban)]
#[rayban(path = AlbumPath, resolved = Resolved)]
struct Album {
    title: String,
}

#[derive(Rayban)]
#[rayban(path = SinglePath, resolved = Resolved)]
struct Single {
    title: String,
}

type AlbumPath<'a> = Path<Album, &'a mut MediaType>;
type SinglePath<'a> = Path<Single, &'a mut MediaType>;

enum Resolved<'a> {
    Album(AlbumPath<'a>),
    Single(SinglePath<'a>),
}

let mut media = MediaType::Single(Single { title: "Bohemian Rhapsody".to_string() });

// Resolve to the active leaf, mutate it.
match <MediaType as Resolve>::resolve(&mut media) {
    Resolved::Single(mut path) => path.get_mut().title.push_str(" (Remastered)"),
    Resolved::Album(_) => unreachable!("built a single"),
}

let MediaType::Single(s) = &media else { unreachable!() };
assert_eq!(s.title, "Bohemian Rhapsody (Remastered)");
```

`#[derive(Rayban)]` implements `Resolve` for each node, so `resolve` on the root walks down the active variants to the one live leaf, returning a `Path`. `get_mut` borrows the focused node; `into_parent` consumes the path and hands you a path focused on the parent one level up.

## Rayban Vision

When iterating, or performing depth first search, or the like through deeply nested data structures, a developer is faced with a variety of bad choices:

- have an enum representing all possible items, throwing away information that e.g. items of type `Album` cannot appear within a `Song`, or
- hand-roll an accurate representation that preserves the information that e.g. an `Album` is not a child of a song.

rayban seeks to provide a better DevEx for the accurate representation.

## License

MIT.

## See also

rayban is the mutable analogue of Isograph's [`resolve_position`](https://github.com/isographlabs/isograph/tree/main/crates/resolve_position). They should be combined.
