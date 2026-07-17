//! Home-layer handlers: the transitions into the other layers. (`q`'s quit is shared with the
//! menu bar; see [`super::quit`].)
//!
//! Every transition sets the layer through `set_layer` and returns its flush. Most are between
//! command layers, so the flush is empty; entering typing (open) and leaving it (close) are the
//! ones that carry effects.
//!
//! Each is generic over the event and the path, so any trigger and any node that ascends to the
//! root can bind it from its own place in the tree.

use bind::Node;
use laserbeam::Ascend;

use super::go_home;
use crate::MercuryEffect;
use crate::state::{AppLayer, MercuryPath, NavLayer, ResizeLayer, TypingLayer};

/// `escape` anywhere, and a layer's idle-timeout: go back to the home layer.
///
/// Typing has to bind `escape` explicitly, because a plain escape passes through there.
pub(crate) fn to_home<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    go_home(node.parent.ascend())
}

/// `n`: enter the nav layer. Bound from home and from the in-app layer.
///
/// Nav arms an idle-timeout, so its constructor also hands back the effect that schedules it.
pub(crate) fn to_nav<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend().set_layer(nav);
    effects.push(timer);
    effects
}

/// `t`: enter the typing layer. Bound from home and from the in-app layer.
pub(crate) fn to_typing<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().set_layer(TypingLayer::new())
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
pub(crate) fn to_inapp<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (inapp, timer) = AppLayer::new();
    let mut effects = node.parent.ascend().set_layer(inapp);
    effects.push(timer);
    effects
}

/// `r` in home: enter the resize layer.
pub(crate) fn to_resize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (resize, timer) = ResizeLayer::new();
    let mut effects = node.parent.ascend().set_layer(resize);
    effects.push(timer);
    effects
}
