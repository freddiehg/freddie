//! Dispatch over the shared tree: leafward descent to the bound handler, the
//! ancestor fallback, the enum active variant, the boxed child, cross-source
//! misses, and the unbound-event `None`. Handlers return the fired key's length,
//! so the return value identifies which one ran.

mod common;

use common::{
    Album, App, Deep, Layer, Media, Nav, Song, TestBindings, Title, Typing, foreground, key,
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
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("g"));
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
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("esc"));
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
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("f1"));
    assert_eq!(out, Some(2)); // "f1"
}

// Through the other enum variant, and on through the boxed `#[resolve_into]`.
#[test]
fn through_typing_variant() {
    let mut app = typing_app();
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("bksp"));
    assert_eq!(out, Some(4)); // "bksp"
    let Layer::Typing(t) = &app.layer else {
        unreachable!()
    };
    assert_eq!(t.hits, 1);
}

#[test]
fn through_box_to_deep() {
    let mut app = typing_app();
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("d"));
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
    let out = bind::dispatch::<TestBindings, App>(&mut app, &foreground("Slack"));
    assert_eq!(out, Some(5)); // "Slack"
}

// An event no node binds returns `None` and mutates nothing.
#[test]
fn unbound_event_is_none() {
    let mut app = nav_app();
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("x"));
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
    let out = bind::dispatch::<TestBindings, App>(&mut app, &key("g"));
    assert_eq!(out, None);
}

// A foreground event that matches no foreground trigger is skipped, not matched
// against the keyboard binds it flows past.
#[test]
fn unmatched_foreground_is_none() {
    let mut app = nav_app();
    let out = bind::dispatch::<TestBindings, App>(&mut app, &foreground("Other"));
    assert_eq!(out, None);
}

// The multi-parent leaf `Title` fires whether reached through `Album` or `Song`.
#[test]
fn multi_parent_leaf_via_album() {
    let mut media = Media::Album(Album {
        title: Title { hits: 0 },
    });
    let out = bind::dispatch::<TestBindings, Media>(&mut media, &key("t"));
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
    let out = bind::dispatch::<TestBindings, Media>(&mut media, &key("t"));
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
    let out = bind::dispatch::<TestBindings, Media>(&mut media, &key("a"));
    assert_eq!(out, Some(1)); // "a"
    let Media::Album(a) = &media else {
        unreachable!()
    };
    assert_eq!(a.title.hits, 0);
}
