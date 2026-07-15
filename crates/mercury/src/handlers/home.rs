//! Home-layer handlers: quit, and the transitions into the other layers.
//!
//! `to_home` also serves every sub-layer, because it is generic over any path that ascends to
//! the layer enum.

use bind::Node;
use freddie_keys::KeyEvent;
use laserbeam::Ascend;

use super::go_home;
use crate::state::{
    AppLayer, HomeLayerPath, Layer, LayerPath, NavLayer, ResizeLayer, TypingLayer,
};
use crate::MercuryEffect;

/// `q` in home: quit.
pub(crate) fn quit(_ev: &KeyEvent, _node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}

/// `escape` anywhere: go back to the home layer.
///
/// Generic over the path, so the layer enum and every node under it can bind it directly.
/// Typing has to bind it explicitly, because its catch-all would otherwise shadow the
/// layer-level binding.
pub(crate) fn to_home<'a, P: Ascend<LayerPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    go_home(&mut node.parent.ascend());
    Vec::new()
}

/// `n`: enter the nav layer. Bound from home and from the in-app layer, so it is
/// generic over any path that ascends to the layer enum, like [`to_home`].
pub(crate) fn to_nav<'a, P: Ascend<LayerPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    *node.parent.ascend().get_mut() = Layer::Nav(NavLayer {});
    Vec::new()
}

/// `t`: enter the typing layer. Generic over the path, so home and the in-app layer
/// both bind it.
pub(crate) fn to_typing<'a, P: Ascend<LayerPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    *node.parent.ascend().get_mut() = Layer::Typing(TypingLayer::default());
    Vec::new()
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
pub(crate) fn to_inapp(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    *node.parent.ascend_to::<LayerPath>().get_mut() = Layer::InApp(AppLayer {});
    Vec::new()
}

/// `r` in home: enter the resize layer.
pub(crate) fn to_resize(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let mut layer = node.parent.ascend_to::<LayerPath>();
    *layer.get_mut() = Layer::Resize(ResizeLayer {});
    Vec::new()
}
