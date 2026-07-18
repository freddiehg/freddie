//! The root's key catch-all: modifier tracking and passthrough, the last resort a key reaches
//! when no layer bound it.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::MercuryEffect;
use crate::effect::emit;
use crate::state::Mercury;

/// Any key the active layer did not bind. A modifier is recorded in `held` (which feeds the
/// open/close sync sweeps) first; its flags are authoritative, and `held` is for the sweeps, not
/// for stamping this. Then pass the key through, carrying exactly the flags it arrived with, while
/// a passthrough layer is active; swallow it otherwise. The source stamped the flags at creation,
/// so a baked-on modifier (an injected `cmd`-`v`, or `fn`) rides along.
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.held.apply(ev);
    }
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}
