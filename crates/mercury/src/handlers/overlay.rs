//! Showing and hiding the overlay: `o` in a layer that binds keys shows its keymap, and the hide
//! timer takes it down.

use bind::Node;
use freddie::TimerFired;
use laserbeam::Ascend;

use crate::MercuryEffect;
use crate::state::{Mercury, MercuryPath};

/// `o` in a layer that binds keys: show that layer's keymap and set its hide timer.
///
/// Generic over the event and the path, so every such layer binds it from its own node.
pub(crate) fn show_overlay<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().show_overlay()
}

/// The overlay's hide timer fired. Bound at the root, so it fires from whatever layer is active,
/// and only for the showing still up: the binding matches the guard the root holds.
pub(crate) fn hide_overlay(_ev: &TimerFired, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.hide_overlay()
}
