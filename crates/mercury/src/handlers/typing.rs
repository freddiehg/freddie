//! Typing-layer handlers: pass keys through while tracking held keys, and exit on cmd-escape.

use bind::Node;
use freddie_keys::{Key, KeyEvent, PressType};

use super::go_home;
use crate::state::TypingLayerPath;
use crate::MercuryEffect;

/// `escape` in typing. If `cmd` is held it exits to home and swallows the escape; otherwise
/// escape is a normal key and passes through.
pub(crate) fn maybe_go_home(
    ev: &KeyEvent,
    mut node: Node<TypingLayerPath, ()>,
) -> Vec<MercuryEffect> {
    let cmd = node.parent.get_mut().held.cmd;
    if cmd {
        let mut layer = node.parent.into_parent();
        go_home(&mut layer);
        Vec::new()
    } else {
        vec![MercuryEffect::Emit(ev.clone())]
    }
}

/// Any key in typing. Update the held set for the keys we track, then pass the key through.
///
/// This is the one handler that fires for every non-escape key, so it is where held-key state
/// is maintained: dispatch runs a single handler per event, and this catch-all is it.
pub(crate) fn modify_held_and_pass_through(
    ev: &KeyEvent,
    mut node: Node<TypingLayerPath, ()>,
) -> Vec<MercuryEffect> {
    if matches!(ev.key, Key::MetaLeft | Key::MetaRight) {
        node.parent.get_mut().held.cmd = ev.press == PressType::Down;
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
