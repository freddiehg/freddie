//! Dispatch over the shared tree: leafward descent to the bound handler, the
//! ancestor fallback, the enum active variant, the boxed child, cross-source
//! misses, and the unbound-event `None`. Handlers return the fired key's length,
//! so the return value identifies which one ran.

mod common;

use common::{
    Album, App, Armed, ArmedChild, Deep, Demo, Layer, Media, Nav, Song, Title, Typing, foreground,
    key,
};

const fn nav_app() -> App {
    App {
        hits: 0,
        layer: Layer::Nav(Nav { hits: 0 }),
    }
}

fn typing_app() -> App {
    App {
        hits: 0,
        layer: Layer::Typing(Typing {
            hits: 0,
            deep: Box::new(Deep { hits: 0 }),
        }),
    }
}

// A binding on the active leaf fires, reached by descending the whole tree.
#[test]
fn leaf_binding_fires() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("g"));
    assert_eq!(out, Some(1)); // "g"
    let Layer::Nav(nav) = &app.layer else {
        unreachable!()
    };
    assert_eq!(nav.hits, 1);
    assert_eq!(app.hits, 0);
}

// `esc` is bound only at the root. The subtree is tried first and misses, so the
// root handler runs: leafward descent, then the ancestor fallback.
#[test]
fn ancestor_binding_fires_after_subtree_misses() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("esc"));
    assert_eq!(out, Some(3)); // "esc"
    assert_eq!(app.hits, 1);
    let Layer::Nav(nav) = &app.layer else {
        unreachable!()
    };
    assert_eq!(nav.hits, 0);
}

// A binding on the enum node itself fires when its active variant misses.
#[test]
fn enum_binding_fires() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("f1"));
    assert_eq!(out, Some(2)); // "f1"
}

// Through the other enum variant, and on through the boxed `#[resolve_into]`.
#[test]
fn through_typing_variant() {
    let mut app = typing_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("bksp"));
    assert_eq!(out, Some(4)); // "bksp"
    let Layer::Typing(t) = &app.layer else {
        unreachable!()
    };
    assert_eq!(t.hits, 1);
}

#[test]
fn through_box_to_deep() {
    let mut app = typing_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("d"));
    assert_eq!(out, Some(1)); // "d"
    let Layer::Typing(t) = &app.layer else {
        unreachable!()
    };
    assert_eq!(t.deep.hits, 1);
}

// A different source's event on the same node.
#[test]
fn foreground_binding_fires() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &foreground("Slack"));
    assert_eq!(out, Some(5)); // "Slack"
}

// An event no node binds returns `None` and mutates nothing.
#[test]
fn unbound_event_is_none() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("x"));
    assert_eq!(out, None);
    assert_eq!(app.hits, 0);
    let Layer::Nav(nav) = &app.layer else {
        unreachable!()
    };
    assert_eq!(nav.hits, 0);
}

// `g` is bound on `Nav`, not `Typing`. With `Typing` active it never fires.
#[test]
fn binding_on_inactive_variant_is_none() {
    let mut app = typing_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &key("g"));
    assert_eq!(out, None);
}

// A foreground event that matches no foreground trigger is skipped, not matched
// against the keyboard binds it flows past.
#[test]
fn unmatched_foreground_is_none() {
    let mut app = nav_app();
    let out = bind::dispatch::<Demo, App>(&mut app, &foreground("Other"));
    assert_eq!(out, None);
}

// The multi-parent leaf `Title` fires whether reached through `Album` or `Song`.
#[test]
fn multi_parent_leaf_via_album() {
    let mut media = Media::Album(Album {
        title: Title { hits: 0 },
    });
    let out = bind::dispatch::<Demo, Media>(&mut media, &key("t"));
    assert_eq!(out, Some(1)); // "t"
    let Media::Album(a) = &media else {
        unreachable!()
    };
    assert_eq!(a.title.hits, 1);
}

#[test]
fn multi_parent_leaf_via_song() {
    let mut media = Media::Song(Song {
        title: Title { hits: 0 },
    });
    let out = bind::dispatch::<Demo, Media>(&mut media, &key("t"));
    assert_eq!(out, Some(1)); // "t"
    let Media::Song(s) = &media else {
        unreachable!()
    };
    assert_eq!(s.title.hits, 1);
}

// The multi-parent ancestor `Album` fires only after `Title` misses, which means
// its path was recovered from the route enum on the way back up.
#[test]
fn multi_parent_ancestor_recover() {
    let mut media = Media::Album(Album {
        title: Title { hits: 0 },
    });
    let out = bind::dispatch::<Demo, Media>(&mut media, &key("a"));
    assert_eq!(out, Some(1)); // "a"
    let Media::Album(a) = &media else {
        unreachable!()
    };
    assert_eq!(a.title.hits, 0);
}

// ---- a trigger that reads the node it is bound on ----

// The closure form: the trigger's value comes from the node, so the same key matches or does not
// depending on what the node is waiting for.
#[test]
fn a_closure_trigger_matches_only_what_its_node_waits_for() {
    let mut armed = Armed {
        waiting_for: Some("g"),
        for_child: None,
        child: ArmedChild { wants: None },
    };
    assert_eq!(
        bind::dispatch::<Demo, Armed>(&mut armed, &key("g")),
        Some(1)
    );
    assert_eq!(armed.waiting_for, None, "the handler ran and cleared it");
}

#[test]
fn a_closure_trigger_matching_nothing_dispatches_nothing() {
    let mut armed = Armed {
        waiting_for: Some("g"),
        for_child: None,
        child: ArmedChild { wants: None },
    };
    // A key it is not waiting for reaches no binding at all: the handler never runs to decline it.
    assert_eq!(bind::dispatch::<Demo, Armed>(&mut armed, &key("h")), None);
    assert_eq!(armed.waiting_for, Some("g"), "nothing was cleared");
}

#[test]
fn a_node_waiting_for_nothing_matches_nothing() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: None },
    };
    assert_eq!(bind::dispatch::<Demo, Armed>(&mut armed, &key("g")), None);
}

#[test]
fn a_constant_trigger_still_works_beside_a_closure_one() {
    let mut armed = Armed {
        waiting_for: Some("g"),
        for_child: None,
        child: ArmedChild { wants: None },
    };
    assert_eq!(
        bind::dispatch::<Demo, Armed>(&mut armed, &key("esc")),
        Some(3)
    );
}

// A deeper node reads through a `PathMut` rather than a `&mut Root`, and its binding wins over the
// root's the way any child's does.
#[test]
fn a_closure_trigger_on_a_deeper_node_reads_through_its_path() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: Some("z") },
    };
    assert_eq!(
        bind::dispatch::<Demo, Armed>(&mut armed, &key("z")),
        Some(1)
    );
    assert_eq!(armed.child.wants, None, "the child's handler ran");
}

// A shared path reads upward too: this binding's trigger comes from the node ABOVE it.
#[test]
fn a_closure_trigger_can_read_its_parent() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: Some("up"),
        child: ArmedChild { wants: None },
    };
    // The child's binding fires for the key its parent named, and the handler that ran says so:
    // the parent-reading one returns the key's length plus 100.
    assert_eq!(
        bind::dispatch::<Demo, Armed>(&mut armed, &key("up")),
        Some(102)
    );
}

// An `Option` trigger: the child binds one, so absence is a value rather than a special case.
#[test]
fn an_absent_option_trigger_matches_nothing() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: None },
    };
    assert_eq!(bind::dispatch::<Demo, Armed>(&mut armed, &key("z")), None);
}

#[test]
fn a_present_option_trigger_matches_its_key() {
    let mut armed = Armed {
        waiting_for: None,
        for_child: None,
        child: ArmedChild { wants: Some("z") },
    };
    assert_eq!(
        bind::dispatch::<Demo, Armed>(&mut armed, &key("z")),
        Some(1)
    );
}
