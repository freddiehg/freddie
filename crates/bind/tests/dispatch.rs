//! A hand-written dispatch, proving the two-level match (extract the source
//! event, then `is_matching`), the leafward descent, per-node handler calls, and
//! `NoHandler`. The path here is a plain `&mut Node`; the derive will build a
//! laserbeam `Path` instead.

use bind::{EventTrigger, FromEvent, NoHandler};

// One source: a keyboard. Its trigger carries a key; its event carries the key
// that fired.
struct Keyboard(&'static str);
struct KeyEvent {
    key: &'static str,
}

impl EventTrigger for Keyboard {
    type Event = KeyEvent;
    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.0 == event.key
    }
}

// The unified event, and the extraction of the source event from it.
enum MercuryEvent {
    Keyboard(KeyEvent),
}

impl FromEvent<MercuryEvent> for KeyEvent {
    fn from_event(event: &MercuryEvent) -> Option<&Self> {
        match event {
            MercuryEvent::Keyboard(k) => Some(k),
        }
    }
}

// A two-node tree: Root holds Child. Each binds a key, and each handler takes
// its own node by `&mut` (the stand-in for a laserbeam path) plus the source
// event, and returns an effect (a number here).
struct Root {
    hits: u32,
    child: Child,
}
struct Child {
    hits: u32,
}

// #[bind(Keyboard("esc") => on_esc)] on Root
const fn on_esc(event: &KeyEvent, root: &mut Root) -> usize {
    root.hits += 1;
    event.key.len()
}

// #[bind(Keyboard("g") => on_g)] on Child
const fn on_g(event: &KeyEvent, child: &mut Child) -> usize {
    child.hits += 1;
    event.key.len()
}

// What the derive will generate per node: check this node's binds, then descend.
fn dispatch_root(root: &mut Root, event: &MercuryEvent) -> Result<usize, NoHandler> {
    if let Some(ev) = FromEvent::from_event(event)
        && Keyboard("esc").is_matching(ev)
    {
        return Ok(on_esc(ev, root));
    }
    dispatch_child(&mut root.child, event)
}

fn dispatch_child(child: &mut Child, event: &MercuryEvent) -> Result<usize, NoHandler> {
    if let Some(ev) = FromEvent::from_event(event)
        && Keyboard("g").is_matching(ev)
    {
        return Ok(on_g(ev, child));
    }
    Err(NoHandler) // a leaf
}

const fn key(k: &'static str) -> MercuryEvent {
    MercuryEvent::Keyboard(KeyEvent { key: k })
}

// A trigger bound at the root fires the root's handler.
#[test]
fn root_binding_fires() {
    let mut root = Root {
        hits: 0,
        child: Child { hits: 0 },
    };
    assert_eq!(dispatch_root(&mut root, &key("esc")), Ok(3));
    assert_eq!(root.hits, 1);
    assert_eq!(root.child.hits, 0);
}

// A trigger bound on the child fires the child's handler, reached by descending.
#[test]
fn child_binding_fires() {
    let mut root = Root {
        hits: 0,
        child: Child { hits: 0 },
    };
    assert_eq!(dispatch_root(&mut root, &key("g")), Ok(1));
    assert_eq!(root.hits, 0);
    assert_eq!(root.child.hits, 1);
}

// An event no node binds returns NoHandler and mutates nothing.
#[test]
fn unbound_event_is_no_handler() {
    let mut root = Root {
        hits: 0,
        child: Child { hits: 0 },
    };
    assert_eq!(dispatch_root(&mut root, &key("x")), Err(NoHandler));
    assert_eq!(root.hits, 0);
    assert_eq!(root.child.hits, 0);
}
