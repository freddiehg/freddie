//! The root's key catch-alls: modifier tracking and passthrough, the last resort a key reaches
//! when no layer bound it.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::effect::emit;
use crate::state::Mercury;
use crate::MercuryEffect;

/// Any modifier key. Record it in `held` (which feeds the open/close sync sweeps), then pass it
/// through while a passthrough layer is active, carrying exactly the flags it arrived with. Its
/// flags are authoritative; `held` is for the sweeps, not for stamping this.
pub(crate) fn on_modifier(ev: &KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.held.apply(ev);
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}

/// Any non-modifier key the active layer did not bind. Pass it through, carrying exactly the flags
/// it arrived with, while a passthrough layer is active; swallow it otherwise. The source stamped
/// the flags at creation, so a baked-on modifier (an injected `cmd`-`v`, or `fn`) rides along.
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}
