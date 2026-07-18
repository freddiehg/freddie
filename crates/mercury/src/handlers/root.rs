//! The root's key catch-all: modifier tracking and passthrough, the last resort a key reaches
//! when no layer bound it.

use bind::Node;
use freddie_keys::{KeyEvent, KeySequenceOutcome};

use crate::MercuryEffect;
use crate::effect::{emit, replay};
use crate::state::{HomeLayer, Mercury};

/// Any key the active layer did not bind.
///
/// A modifier is recorded in `held` (which feeds the open/close sync sweeps) first; its flags are
/// authoritative, and `held` is for the sweeps, not for stamping this. Outside a passthrough layer
/// the key is swallowed and that is all.
///
/// In a passthrough layer the key goes to the `jk` run first, which either takes it, hands back
/// what it had swallowed for a key that broke it, or completes and leaves for home. A key the run
/// does not want is passed through carrying exactly the flags it arrived with, so a baked-on
/// modifier (an injected `cmd`-`v`, or `fn`) rides along.
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    if !root.layer().is_passthrough() {
        return Vec::new();
    }
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced => Vec::new(),
        KeySequenceOutcome::Passed(presses) => {
            let mut out = replay(presses);
            out.push(emit(ev.key, ev.press, ev.flags));
            out
        }
        KeySequenceOutcome::Completed => root.set_layer(HomeLayer::new()),
    }
}
