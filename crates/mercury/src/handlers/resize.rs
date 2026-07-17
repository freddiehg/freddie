//! Resize-layer handlers: place the focused window and return home.

use bind::Node;
use laserbeam::Ascend;

use super::and_go_home;
use crate::state::MercuryPath;
use crate::{MercuryEffect, Placement};

pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::Maximize)])
}
pub(crate) fn left_half<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::LeftHalf)])
}
pub(crate) fn right_half<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(
        node.parent,
        vec![MercuryEffect::Place(Placement::RightHalf)],
    )
}
