//! Accumulation over the shared tree: through an enum, through a `#[resolve_into]`
//! field (boxed and non-boxed), the duplicate-trigger error, and a no-binds node.

mod common;

use std::collections::HashSet;

use common::{
    App, Armed, ArmedChild, Clash, ClashChild, Deep, Demo, Empty, Layer, Nav, Typing, fg, kb,
    waiting,
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

// THE CHECK evaluates a trigger too, so a closure one has to be called there as well. This is the
// only test of that half: without it the accumulate emit breaks silently for anyone enabling the
// feature.
#[test]
fn a_closure_trigger_is_collected_as_the_value_it_produced() {
    let mut armed = Armed {
        waiting_for: Some("g"),
        for_child: None,
        child: ArmedChild { wants: Some("z") },
    };
    let set = bind::accumulate::<Demo, Armed>(&mut armed).unwrap();
    assert_eq!(
        set,
        HashSet::from([
            waiting(Some("g")),
            waiting(Some("z")),
            kb("esc"),
            // The child's other binding reads its PARENT, so the trigger it contributes comes from
            // the root: nothing set, so the stand-in.
            kb("none"),
        ])
    );
}

// A node waiting for nothing still contributes a trigger; it is one that matches no event.
#[test]
fn an_unarmed_closure_trigger_is_collected_as_none() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: Some("z") },
    };
    let set = bind::accumulate::<Demo, Armed>(&mut armed).unwrap();
    assert!(set.contains(&waiting(None)));
}
