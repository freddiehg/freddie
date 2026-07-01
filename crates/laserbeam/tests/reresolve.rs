//! Resolve depth-first to the active leaf, walk up via the path, swap the
//! active variant in place (through the path, not by rebuilding from the root),
//! then re-resolve from that same subtree path onto the now-active leaf.
//!
//! Tree: `Catalog -> MediaType -> {Album, Single}`.

use laserbeam::{Laserbeam, Path, Resolve};

#[derive(Laserbeam)]
#[laserbeam_root(resolved = Resolved)]
struct Catalog {
    #[resolve_into]
    featured: MediaType,
}

#[derive(Laserbeam)]
#[laserbeam(path = MediaTypePath, resolved = Resolved)]
enum MediaType {
    Album(Album),
    Single(Single),
}

#[derive(Laserbeam)]
#[laserbeam(path = AlbumPath, resolved = Resolved)]
struct Album {
    title: String,
}

#[derive(Laserbeam)]
#[laserbeam(path = SinglePath, resolved = Resolved)]
struct Single {
    title: String,
}

type MediaTypePath<'a> = Path<MediaType, &'a mut Catalog>;
type AlbumPath<'a> = Path<Album, MediaTypePath<'a>>;
type SinglePath<'a> = Path<Single, MediaTypePath<'a>>;

enum Resolved<'a> {
    Album(AlbumPath<'a>),
    Single(SinglePath<'a>),
}

#[test]
fn resolve_walk_up_swap_variant_then_reresolve() {
    let mut catalog = Catalog {
        featured: MediaType::Single(Single {
            title: "Bohemian Rhapsody".to_string(),
        }),
    };

    // Resolve depth-first; the active leaf is a Single. Walk up to its parent's
    // path (the MediaType).
    let mut media = match <Catalog as Resolve>::resolve(&mut catalog) {
        Resolved::Single(mut single) => {
            assert_eq!(single.get_mut().title, "Bohemian Rhapsody");
            single.into_parent()
        }
        Resolved::Album(_) => unreachable!("featured a single"),
    };

    // Mutate the parent through its path: swap the active variant to an Album.
    // The `&mut Catalog` at the bottom of the path is never re-borrowed from the
    // root; we only ever hold this one live `&mut`.
    *media.get_mut() = MediaType::Album(Album {
        title: "A Night at the Opera".to_string(),
    });

    // Re-resolve from the same MediaType path. The DFS now lands on the Album.
    match <MediaType as Resolve>::resolve(media) {
        Resolved::Album(mut album) => album.get_mut().title.push_str(" (Remastered)"),
        Resolved::Single(_) => unreachable!("just swapped to an album"),
    }

    let MediaType::Album(a) = &catalog.featured else {
        unreachable!()
    };
    assert_eq!(a.title, "A Night at the Opera (Remastered)");
}
