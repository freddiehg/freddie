//! Home-layer handlers: the transitions into the other layers. (`q`'s quit is shared with the
//! menu bar; see [`super::quit`].)
//!
//! Every transition sets the layer through `set_layer` and returns its flush. Most are between
//! command layers, so the flush is empty; entering typing (open) and leaving it (close) are the
//! ones that carry effects.

use bind::Node;
use freddie_keys::KeyEvent;
use laserbeam::Ascend;

use super::go_home;
use crate::MercuryEffect;
use crate::state::{AppLayer, HomeLayerPath, MercuryPath, NavLayer, ResizeLayer, TypingLayer};

/// `escape` anywhere: go back to the home layer.
///
/// Generic over the path, so the layer enum and every node under it can bind it directly.
/// Typing has to bind it explicitly, because a plain escape passes through there.
pub(crate) fn to_home<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    go_home(node.parent.ascend())
}

/// `n`: enter the nav layer. Bound from home and from the in-app layer, so it is
/// generic over any path that ascends to the root, like [`to_home`].
pub(crate) fn to_nav<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().set_layer(NavLayer::new())
}

/// `t`: enter the typing layer. Generic over the path, so home and the in-app layer
/// both bind it.
pub(crate) fn to_typing<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().set_layer(TypingLayer::new())
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
pub(crate) fn to_inapp(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    node.parent
        .ascend_to::<MercuryPath>()
        .set_layer(AppLayer::new())
}

/// `r` in home: enter the resize layer.
pub(crate) fn to_resize(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    node.parent
        .ascend_to::<MercuryPath>()
        .set_layer(ResizeLayer::new())
}
