//! The quit source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, QuitEvent};

/// A quit was requested (the menu bar's Quit): kill the program.
///
/// Bound at the root, so it fires from any layer. That is the point: the menu-bar
/// Quit is a recovery path, and it must work whatever layer the model is in, unlike
/// `q`, which quits only from home.
pub(crate) fn on_quit(_ev: &QuitEvent, _node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}
