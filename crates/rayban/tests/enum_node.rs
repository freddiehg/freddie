//! Validates the non-root enum case: an enum node in the middle of the tree,
//! where the macro emits the `matches!` / early-return descent.

use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = Resolved)]
enum Root {
    Only(Mid),
}

#[derive(Rayban)]
#[rayban(path = MidPath, resolved = Resolved)]
enum Mid {
    Left(LeftLeaf),
    Right(RightLeaf),
}

#[derive(Rayban)]
#[rayban(path = LeftPath, resolved = Resolved)]
struct LeftLeaf {
    value: u32,
}

#[derive(Rayban)]
#[rayban(path = RightPath, resolved = Resolved)]
struct RightLeaf {
    name: String,
}

type MidPath<'a> = Path<Mid, &'a mut Root>;
type LeftPath<'a> = Path<LeftLeaf, MidPath<'a>>;
type RightPath<'a> = Path<RightLeaf, MidPath<'a>>;

enum Resolved<'a> {
    LeftLeaf(LeftPath<'a>),
    RightLeaf(RightPath<'a>),
}

#[test]
fn descends_through_a_non_root_enum() {
    let mut root = Root::Only(Mid::Left(LeftLeaf { value: 1 }));
    match <Root as Resolve>::resolve(&mut root) {
        Resolved::LeftLeaf(mut p) => {
            p.get_mut().value += 41;
            let _ = p.into_parent(); // walk up one: LeftLeaf -> Mid's path
        }
        Resolved::RightLeaf(_) => unreachable!("built a Left"),
    }
    let Root::Only(Mid::Left(leaf)) = &root else {
        unreachable!()
    };
    assert_eq!(leaf.value, 42);
}

#[test]
fn picks_the_active_variant() {
    let mut root = Root::Only(Mid::Right(RightLeaf {
        name: "x".to_owned(),
    }));
    match <Root as Resolve>::resolve(&mut root) {
        Resolved::RightLeaf(mut p) => p.get_mut().name.push('!'),
        Resolved::LeftLeaf(_) => unreachable!("built a Right"),
    }
    let Root::Only(Mid::Right(leaf)) = &root else {
        unreachable!()
    };
    assert_eq!(leaf.name, "x!");
}
