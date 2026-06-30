//! The same tree as `tree.rs`, but built with `#[derive(Rayban)]`. This proves
//! the macro emits the impls the hand-written test spells out, and compiles in a
//! separate crate from `rayban` (so the generated `From` impls are orphan-safe).

use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = MediaResolved)]
enum MediaType {
    Album(Album),
    Song(Song),
}

#[derive(Rayban)]
#[rayban(path = AlbumPath, resolved = MediaResolved)]
struct Album {
    year: u16,
    #[resolve_into(parent = TitleParent)]
    title: Title,
}

#[derive(Rayban)]
#[rayban(path = SongPath, resolved = MediaResolved)]
struct Song {
    #[resolve_into(parent = TitleParent)]
    title: Title,
}

#[derive(Rayban)]
#[rayban(path = TitlePath, resolved = MediaResolved)]
struct Title {
    #[resolve_into]
    credit: Credit,
}

#[derive(Rayban)]
#[rayban(path = CreditPath, resolved = MediaResolved)]
struct Credit {
    name: String,
}

// Single-parent paths.
type AlbumPath<'a> = Path<Album, &'a mut MediaType>;
type SongPath<'a> = Path<Song, &'a mut MediaType>;

// Title has two parents, so its parent is a plain route enum (one variant per
// parent path) and its path is an ordinary `Path<Title, TitleParent>`. No derive
// needed: `get_mut`/`into_parent` come from `Path`.
enum TitleParent<'a> {
    Album(AlbumPath<'a>),
    Song(SongPath<'a>),
}

type TitlePath<'a> = Path<Title, TitleParent<'a>>;
type CreditPath<'a> = Path<Credit, TitlePath<'a>>;

enum MediaResolved<'a> {
    Credit(CreditPath<'a>),
}

fn album(name: &str) -> MediaType {
    MediaType::Album(Album {
        year: 1975,
        title: Title {
            credit: Credit {
                name: name.to_owned(),
            },
        },
    })
}

fn song(name: &str) -> MediaType {
    MediaType::Song(Song {
        title: Title {
            credit: Credit {
                name: name.to_owned(),
            },
        },
    })
}

#[test]
fn resolves_through_album_mutates_leaf_and_ancestor() {
    let mut media = album("Roger");
    match <MediaType as Resolve>::resolve(&mut media) {
        MediaResolved::Credit(mut cp) => {
            cp.get_mut().name.push_str(" Taylor"); // mutate the leaf
            // up: Credit -> Title's path -> Title's parent -> which parent, then
            // mutate the ancestor's own field.
            match cp.into_parent().into_parent() {
                TitleParent::Album(mut ap) => ap.get_mut().year += 1,
                TitleParent::Song(_) => unreachable!("resolved through an album"),
            }
        }
    }
    let MediaType::Album(a) = &media else {
        unreachable!()
    };
    assert_eq!(a.title.credit.name, "Roger Taylor");
    assert_eq!(a.year, 1976);
}

#[test]
fn resolves_through_song_picks_the_song_parent() {
    let mut media = song("Freddie");
    match <MediaType as Resolve>::resolve(&mut media) {
        MediaResolved::Credit(cp) => match cp.into_parent().into_parent() {
            TitleParent::Song(mut sp) => {
                assert_eq!(sp.get_mut().title.credit.name, "Freddie");
            }
            TitleParent::Album(_) => unreachable!("resolved through a song"),
        },
    }
}

#[test]
fn resolve_mutate_reresolve_round_trip() {
    let mut media = album("a");
    // Resolve, mutate, drop the path (releasing the borrow).
    match <MediaType as Resolve>::resolve(&mut media) {
        MediaResolved::Credit(mut cp) => cp.get_mut().name.push_str("bc"),
    }
    // Resolve again: the mutation persisted, and a fresh path works.
    match <MediaType as Resolve>::resolve(&mut media) {
        MediaResolved::Credit(mut cp) => {
            assert_eq!(cp.get_mut().name, "abc");
            cp.get_mut().name.push('!');
        }
    }
    let MediaType::Album(a) = &media else {
        unreachable!()
    };
    assert_eq!(a.title.credit.name, "abc!");
}
