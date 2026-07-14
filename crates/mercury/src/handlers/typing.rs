//! Typing-layer catch-all.

use freddie_keys::KeyEvent;

use crate::MercuryEffect;

/// Any key that is not `escape`: pass it straight through.
pub(crate) fn passthru<P>(ev: &KeyEvent, _node: P) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}
