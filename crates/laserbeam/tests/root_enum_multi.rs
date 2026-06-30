//! A root enum descending into a multi-parent child. `Track` is reached from the
//! root `Discography` (the `Single` variant) and from `Record.opener` (a struct
//! field), so it is multi-parent. This exercises the is-root enum-variant
//! multi-parent descent.

use laserbeam::{Path, Laserbeam, Resolve};

#[derive(Laserbeam)]
#[laserbeam_root(resolved = Resolved)]
enum Discography {
    #[resolve_into(parent = TrackParent)]
    Single(Track),
    Album(Record),
}

#[derive(Laserbeam)]
#[laserbeam(path = RecordPath, resolved = Resolved)]
struct Record {
    #[resolve_into(parent = TrackParent)]
    opener: Track,
}

#[derive(Laserbeam)]
#[laserbeam(path = TrackPath, resolved = Resolved)]
struct Track {
    title: String,
}

enum TrackParent<'a> {
    Discography(&'a mut Discography),
    Record(Box<RecordPath<'a>>),
}

type RecordPath<'a> = Path<Record, &'a mut Discography>;
type TrackPath<'a> = Path<Track, TrackParent<'a>>;

enum Resolved<'a> {
    Track(TrackPath<'a>),
}

#[test]
fn root_enum_into_multi_parent_via_single() {
    let mut disco = Discography::Single(Track {
        title: "Bohemian Rhapsody".to_owned(),
    });
    match <Discography as Resolve>::resolve(&mut disco) {
        Resolved::Track(mut p) => p.get_mut().title.push_str(" (Remastered)"),
    }
    let Discography::Single(t) = &disco else { unreachable!() };
    assert_eq!(t.title, "Bohemian Rhapsody (Remastered)");
}

#[test]
fn root_enum_into_multi_parent_via_album_opener() {
    let mut disco = Discography::Album(Record {
        opener: Track {
            title: "Death on Two Legs".to_owned(),
        },
    });
    match <Discography as Resolve>::resolve(&mut disco) {
        Resolved::Track(mut p) => p.get_mut().title.push('!'),
    }
    let Discography::Album(r) = &disco else { unreachable!() };
    assert_eq!(r.opener.title, "Death on Two Legs!");
}
