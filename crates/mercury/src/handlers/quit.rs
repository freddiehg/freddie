//! The quit source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, QuitEvent};

/// A quit was requested (the menu bar's Quit): kill the program.
///
/// Bound at the root, so it fires from any layer. That is the point: the menu-bar
/// Quit is a recovery path, and it must work whatever layer the model is in, unlike
/// `q`, which quits only from home.
///
/// Emit the held modifiers' downs first. In a command layer their real downs were swallowed, so
/// the app does not know they are held; once the grab is released no further down is coming, so
/// tell it now, before `Kill`, or it is left thinking a physically-held modifier is up.
pub(crate) fn on_quit(_ev: &QuitEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    let mut effects = root.held.open();
    effects.push(MercuryEffect::Kill);
    effects
}
