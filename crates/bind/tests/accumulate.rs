//! Accumulation over a state tree: through an enum, through a `#[resolve_into]`
//! field (boxed and non-boxed), and the duplicate-trigger error.

use std::collections::HashSet;

use bind::{Bind, BindError, Bindings};

// Two source types, hand-written `From` into the unified trigger enum.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Keyboard(&'static str);
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Foreground(&'static str);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum MercuryTrigger {
    Keyboard(Keyboard),
    Foreground(Foreground),
}
impl From<Keyboard> for MercuryTrigger {
    fn from(k: Keyboard) -> Self {
        Self::Keyboard(k)
    }
}
impl From<Foreground> for MercuryTrigger {
    fn from(f: Foreground) -> Self {
        Self::Foreground(f)
    }
}

struct MercuryStruct;
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
}

#[allow(dead_code)]
const fn noop() {}

// App -> Layer (enum) -> { Nav (leaf), Typing -> Box<Deep> (leaf) }.
#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("esc") => noop)]
struct App {
    #[resolve_into]
    layer: Layer,
}

#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("f1") => noop)]
enum Layer {
    Nav(Nav),
    Typing(Typing),
}

#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("g") => noop, Foreground("Slack") => noop)]
struct Nav {}

#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("bksp") => noop)]
struct Typing {
    #[resolve_into]
    deep: Box<Deep>,
}

#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("d") => noop)]
struct Deep {}

const fn kb(s: &'static str) -> MercuryTrigger {
    MercuryTrigger::Keyboard(Keyboard(s))
}
const fn fg(s: &'static str) -> MercuryTrigger {
    MercuryTrigger::Foreground(Foreground(s))
}

// Through the Layer enum and the non-boxed `#[resolve_into]` App -> Layer.
#[test]
fn through_enum_and_resolve_into() {
    let app = App {
        layer: Layer::Nav(Nav {}),
    };
    let set = bind::accumulate::<MercuryStruct, _>(&app).unwrap();
    assert_eq!(
        set,
        HashSet::from([kb("esc"), kb("f1"), kb("g"), fg("Slack")])
    );
}

// Through the boxed `#[resolve_into]` Typing -> Box<Deep>.
#[test]
fn through_boxed_resolve_into() {
    let app = App {
        layer: Layer::Typing(Typing {
            deep: Box::new(Deep {}),
        }),
    };
    let set = bind::accumulate::<MercuryStruct, _>(&app).unwrap();
    assert_eq!(
        set,
        HashSet::from([kb("esc"), kb("f1"), kb("bksp"), kb("d")])
    );
}

// A child rebinding an ancestor's trigger is an error.
#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("dup") => noop)]
struct Clash {
    #[resolve_into]
    child: ClashChild,
}

#[derive(Bind)]
#[binds(MercuryStruct)]
#[bind(Keyboard("dup") => noop)]
struct ClashChild {}

#[test]
fn duplicate_trigger_is_error() {
    let clash = Clash {
        child: ClashChild {},
    };
    assert_eq!(
        bind::accumulate::<MercuryStruct, _>(&clash),
        Err(BindError::DuplicateTrigger)
    );
}

// A node with no `#[bind]` is fine; it accumulates nothing.
#[derive(Bind)]
#[binds(MercuryStruct)]
struct Empty {}

#[test]
fn no_binds_is_empty() {
    let set = bind::accumulate::<MercuryStruct, _>(&Empty {}).unwrap();
    assert!(set.is_empty());
}
