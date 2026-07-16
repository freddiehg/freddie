//! The root's key catch-alls: modifier tracking and passthrough, the last resort a key reaches
//! when no layer bound it.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::effect::emit;
use crate::state::Mercury;
use crate::MercuryEffect;

/// Any modifier key. Record it in `held` (always, in every layer, so `held` stays accurate), then
/// pass it through while a passthrough layer is active; swallow it in a command layer.
pub(crate) fn on_modifier(ev: &KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.held.apply(ev);
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, root.held.flags().union(ev.flags))]
    } else {
        Vec::new()
    }
}

/// Any non-modifier key the active layer did not bind. Pass it through while a passthrough layer
/// is active; swallow it otherwise.
///
/// The emitted flags are the tracked held modifiers UNION the modifiers baked onto this event, so
/// a modifier that never arrived as its own key (an injected `cmd`-`v`, or `fn`) still rides along.
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, root.held.flags().union(ev.flags))]
    } else {
        Vec::new()
    }
}
