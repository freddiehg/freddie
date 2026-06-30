//! A recursive tree from the "Bismillah" call-and-response of Bohemian Rhapsody.
//! The cycle is
//!
//!   ask to let go    -> bismillah                      (will you let me go?)
//!   bismillah        -> refuse to let go               (Bismillah!)
//!   refuse to let go -> ask to let go | never let me go (we will not let you go | ...)
//!   never let me go  -> leaf                           (never, never, never let me go)
//!
//! so `AskToLetGo` is multi-parent: reached from the root (`Opera`, a struct
//! field) and from `RefuseToLetGo` (an enum variant). The latter is the
//! enum-node-to-multi-parent edge, with the recursion boxed to stay finite.

use rayban::{Path, Rayban, Resolve};

#[derive(Rayban)]
#[rayban_root(resolved = Resolved)]
struct Opera {
    #[resolve_into(parent = AskToLetGoParent)]
    verse: AskToLetGo,
}

#[derive(Rayban)]
#[rayban(path = AskToLetGoPath, resolved = Resolved)]
struct AskToLetGo {
    #[resolve_into]
    plea: Bismillah,
}

#[derive(Rayban)]
#[rayban(path = BismillahPath, resolved = Resolved)]
struct Bismillah {
    #[resolve_into]
    refusal: RefuseToLetGo,
}

#[derive(Rayban)]
#[rayban(path = RefuseToLetGoPath, resolved = Resolved)]
enum RefuseToLetGo {
    #[resolve_into(parent = AskToLetGoParent)]
    AskToLetGo(Box<AskToLetGo>), // recursion via an enum variant; boxed
    NeverLetMeGo(NeverLetMeGo),
}

#[derive(Rayban)]
#[rayban(path = NeverLetMeGoPath, resolved = Resolved)]
struct NeverLetMeGo {
    cry: String,
}

enum AskToLetGoParent<'a> {
    Opera(&'a mut Opera),
    RefuseToLetGo(Box<RefuseToLetGoPath<'a>>), // boxed: the path chain is recursive too
}

type AskToLetGoPath<'a> = Path<AskToLetGo, AskToLetGoParent<'a>>;
type BismillahPath<'a> = Path<Bismillah, AskToLetGoPath<'a>>;
type RefuseToLetGoPath<'a> = Path<RefuseToLetGo, BismillahPath<'a>>;
type NeverLetMeGoPath<'a> = Path<NeverLetMeGo, RefuseToLetGoPath<'a>>;

enum Resolved<'a> {
    NeverLetMeGo(NeverLetMeGoPath<'a>),
}

#[test]
fn resolves_through_recursive_refusal() {
    // ask -> bismillah -> refuse -> ask -> bismillah -> refuse -> never let me go
    let mut opera = Opera {
        verse: AskToLetGo {
            plea: Bismillah {
                refusal: RefuseToLetGo::AskToLetGo(Box::new(AskToLetGo {
                    plea: Bismillah {
                        refusal: RefuseToLetGo::NeverLetMeGo(NeverLetMeGo {
                            cry: "never, never, never, never let me go".to_owned(),
                        }),
                    },
                })),
            },
        },
    };

    match <Opera as Resolve>::resolve(&mut opera) {
        Resolved::NeverLetMeGo(mut p) => p.get_mut().cry.push_str(" No, no, no, no, no, no"),
    }

    let RefuseToLetGo::AskToLetGo(ask2) = &opera.verse.plea.refusal else {
        unreachable!()
    };
    let RefuseToLetGo::NeverLetMeGo(leaf) = &ask2.plea.refusal else {
        unreachable!()
    };
    assert_eq!(
        leaf.cry,
        "never, never, never, never let me go No, no, no, no, no, no"
    );
}
