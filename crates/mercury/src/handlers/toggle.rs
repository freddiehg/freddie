//! The toggle source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, ToggleEvent};

/// A toggle was requested (the menu bar's Toggle): flip `enabled`.
///
/// Bound at the root, so it fires from any layer. Nothing reads `enabled` yet, so this only
/// records the state; gating the layer on it is `enable-disable.md`.
pub(crate) const fn on_toggle(_ev: &ToggleEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.power.toggle();
    Vec::new()
}
