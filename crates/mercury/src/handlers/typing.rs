//! Typing-layer handler: leave to home on cmd-escape, otherwise pass the escape through.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::MercuryEffect;
use crate::effect::emit;
use crate::state::{HomeLayer, MercuryPath, TypingLayerPath};

/// `escape` in typing. If `cmd` is held it leaves to home, and `set_layer`'s close releases every
/// held modifier (the escape itself is swallowed). Otherwise the escape passes through: it is a
/// bound key, so it clobbered the root passthrough and typing has to re-emit it by hand.
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.held.meta.any_held() {
        root.set_layer(HomeLayer::new())
    } else {
        vec![emit(ev.key, ev.press, root.held.flags())]
    }
}
