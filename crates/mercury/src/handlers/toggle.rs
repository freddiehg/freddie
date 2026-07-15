//! The toggle source's handler, and the disabled arm's passthrough.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::state::{DisabledPath, Mercury};
use crate::{MercuryEffect, ToggleEvent};

/// A toggle was requested (the menu bar's Toggle): flip enabled/disabled.
///
/// Bound at the root, so it fires whether or not the layer is descended into. When disabled the
/// layer's binds are off, so this menu-driven toggle is the way back on (along with any future
/// re-enable chord).
pub(crate) const fn on_toggle(_ev: &ToggleEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.power.toggle();
    Vec::new()
}

/// Any key while disabled: pass it through untouched. The disabled arm does not descend into the
/// layer, so this catch-all is what every key reaches, and re-emitting it gives a normal keyboard.
pub(crate) fn pass_through(ev: &KeyEvent, _node: Node<DisabledPath, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}
