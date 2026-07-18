//! Typing-layer handler: leave to home on cmd-escape, otherwise pass the escape through.

use bind::Node;
use freddie_keys::KeyEvent;
use laserbeam::Ascend;

use crate::MercuryEffect;
use crate::effect::emit;
use crate::state::{HomeLayer, MercuryPath};

/// `escape` in typing. If `cmd` is held it leaves to home, and `set_layer`'s close releases every
/// held modifier (the escape itself is swallowed). Otherwise the escape passes through: it is a
/// bound key, so it clobbered the root passthrough and typing has to re-emit it by hand.
///
/// Reads the event, so the type stays concrete; generic over the path.
pub(crate) fn maybe_go_home<'a, P: Ascend<MercuryPath<'a>>>(
    ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = node.parent.ascend();
    if root.typing_state.held.meta.any_held() {
        root.set_layer(HomeLayer::new())
    } else {
        vec![emit(ev.key, ev.press, root.typing_state.held.flags())]
    }
}
