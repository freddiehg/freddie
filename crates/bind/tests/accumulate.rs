//! Accumulation over the shared tree: through an enum, through a `#[resolve_into]`
//! field (boxed and non-boxed), the duplicate-trigger error, and a no-binds node.

mod common;

use std::collections::HashSet;

use common::{App, Clash, ClashChild, Deep, Empty, Layer, MercuryStruct, Nav, Typing, fg, kb};

// Through the Layer enum and the non-boxed `#[resolve_into]` App -> Layer.
#[test]
fn through_enum_and_resolve_into() {
    let mut app = App {
        hits: 0,
        layer: Layer::Nav(Nav { hits: 0 }),
    };
    let set = bind::accumulate::<MercuryStruct, App>(&mut app).unwrap();
    assert_eq!(
        set,
        HashSet::from([kb("esc"), kb("f1"), kb("g"), fg("Slack")])
    );
}

// Through the boxed `#[resolve_into]` Typing -> Box<Deep>.
#[test]
fn through_boxed_resolve_into() {
    let mut app = App {
        hits: 0,
        layer: Layer::Typing(Typing {
            hits: 0,
            deep: Box::new(Deep { hits: 0 }),
        }),
    };
    let set = bind::accumulate::<MercuryStruct, App>(&mut app).unwrap();
    assert_eq!(
        set,
        HashSet::from([kb("esc"), kb("f1"), kb("bksp"), kb("d")])
    );
}

// A child rebinding an ancestor's trigger is an error.
#[test]
fn duplicate_trigger_is_error() {
    let mut clash = Clash {
        child: ClashChild {},
    };
    assert_eq!(
        bind::accumulate::<MercuryStruct, Clash>(&mut clash),
        Err(bind::BindError::DuplicateTrigger)
    );
}

// A node with no `#[bind]` is fine; it accumulates nothing.
#[test]
fn no_binds_is_empty() {
    let set = bind::accumulate::<MercuryStruct, Empty>(&mut Empty {}).unwrap();
    assert!(set.is_empty());
}
