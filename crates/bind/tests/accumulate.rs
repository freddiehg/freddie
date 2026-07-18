//! Accumulation over the shared tree: through an enum, through a `#[resolve_into]`
//! field (boxed and non-boxed), the duplicate-trigger error, and a no-binds node.

mod common;

use std::collections::HashSet;

use common::{
    App, Armed, ArmedChild, Clash, ClashChild, Deep, Demo, Empty, Layer, Nav, Typing, fg, kb,
};

// Through the Layer enum and the non-boxed `#[resolve_into]` App -> Layer.
#[test]
fn through_enum_and_resolve_into() {
    let mut app = App {
        hits: 0,
        layer: Layer::Nav(Nav { hits: 0 }),
    };
    let set = bind::accumulate::<Demo, App>(&mut app).unwrap();
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
    let set = bind::accumulate::<Demo, App>(&mut app).unwrap();
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
        bind::accumulate::<Demo, Clash>(&mut clash),
        Err(bind::BindError::DuplicateTrigger)
    );
}

// A node with no `#[bind]` is fine; it accumulates nothing.
#[test]
fn no_binds_is_empty() {
    let set = bind::accumulate::<Demo, Empty>(&mut Empty {}).unwrap();
    assert!(set.is_empty());
}

// THE CHECK collects what a node CLAIMS, and a closure trigger's value is read from state at
// dispatch, so it is not one. Skipping them is also what lets such a trigger be an `Option`: there
// is no value to insert for an absent one.
#[test]
fn a_closure_trigger_is_not_collected() {
    let mut armed = Armed {
        waiting_for: Some("g"),
        for_child: Some("up"),
        child: ArmedChild { wants: Some("z") },
    };
    let set = bind::accumulate::<Demo, Armed>(&mut armed).unwrap();
    // Only the constant trigger on the root; the three closure ones contribute nothing, whatever
    // their state says.
    assert_eq!(set, HashSet::from([kb("esc")]));
}

// Two nodes holding nothing would produce the same trigger value, which the set would have read as
// one node clobbering the other. Skipped, they cannot.
#[test]
fn two_nodes_with_nothing_to_match_are_not_a_duplicate() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: None },
    };
    assert!(bind::accumulate::<Demo, Armed>(&mut armed).is_ok());
}
