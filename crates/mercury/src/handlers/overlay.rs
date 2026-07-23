//! Showing and hiding the overlay: `o` in a layer that binds keys toggles its keymap, and the
//! hide timer takes it down on its own.

use bind::Node;
use freddie::TimerFired;
use laserbeam::AscendMut;

use crate::MercuryEffect;
use crate::state::{Mercury, MercuryPath};

/// `o` in a layer that binds keys: show that layer's keymap, or take it down if it is up.
///
/// Generic over the event and the path, so every such layer binds it from its own node.
pub(crate) fn toggle_overlay<'a, E, P: AscendMut<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend_mut().toggle_overlay()
}

/// The overlay's hide timer fired. Bound at the root, so it fires from whatever layer is active,
/// and only for the showing still up: the binding matches the guard the root holds.
pub(crate) fn hide_overlay(_ev: &TimerFired, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.hide_overlay()
}
