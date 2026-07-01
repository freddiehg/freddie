//! Validates the non-root enum case: an enum node in the middle of the tree,
//! where the macro emits the `matches!` / early-return descent. Modeled on
//! Queen II, pressed as "Side White" and "Side Black".

use laserbeam::{Laserbeam, Path, Resolve};

#[derive(Laserbeam)]
#[laserbeam_root(resolved = Resolved)]
enum QueenII {
    Pressing(Side),
}

#[derive(Laserbeam)]
#[laserbeam(path = SidePath, resolved = Resolved)]
enum Side {
    White(WhiteQueen),
    Black(BlackQueen),
}

#[derive(Laserbeam)]
#[laserbeam(path = WhiteQueenPath, resolved = Resolved)]
struct WhiteQueen {
    bpm: u32,
}

#[derive(Laserbeam)]
#[laserbeam(path = BlackQueenPath, resolved = Resolved)]
struct BlackQueen {
    title: String,
}

type SidePath<'a> = Path<Side, &'a mut QueenII>;
type WhiteQueenPath<'a> = Path<WhiteQueen, SidePath<'a>>;
type BlackQueenPath<'a> = Path<BlackQueen, SidePath<'a>>;

enum Resolved<'a> {
    WhiteQueen(WhiteQueenPath<'a>),
    BlackQueen(BlackQueenPath<'a>),
}

#[test]
fn descends_through_a_non_root_enum() {
    let mut record = QueenII::Pressing(Side::White(WhiteQueen { bpm: 1 }));
    match <QueenII as Resolve>::resolve(&mut record) {
        Resolved::WhiteQueen(mut p) => {
            p.get_mut().bpm += 41;
            let _ = p.into_parent(); // walk up one: WhiteQueen -> Side's path
        }
        Resolved::BlackQueen(_) => unreachable!("pressed Side White"),
    }
    let QueenII::Pressing(Side::White(track)) = &record else {
        unreachable!()
    };
    assert_eq!(track.bpm, 42);
}

#[test]
fn picks_the_active_variant() {
    let mut record = QueenII::Pressing(Side::Black(BlackQueen {
        title: "The March of the Black Queen".to_owned(),
    }));
    match <QueenII as Resolve>::resolve(&mut record) {
        Resolved::BlackQueen(mut p) => p.get_mut().title.push('!'),
        Resolved::WhiteQueen(_) => unreachable!("pressed Side Black"),
    }
    let QueenII::Pressing(Side::Black(track)) = &record else {
        unreachable!()
    };
    assert_eq!(track.title, "The March of the Black Queen!");
}
