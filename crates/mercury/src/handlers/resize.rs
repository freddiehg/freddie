//! Resize-layer handlers: place the focused window and return home.

use bind::Node;
use freddie_keys::KeyEvent;

use super::and_go_home;
use crate::tree::ResizeLayerPath;
use crate::{MercuryEffect, Placement};

pub(crate) fn maximize(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::Maximize)])
}
pub(crate) fn left_half(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::LeftHalf)])
}
pub(crate) fn right_half(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::RightHalf)])
}
