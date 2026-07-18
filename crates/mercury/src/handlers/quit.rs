//! The program's one way out, shared by home's `q` and the menu bar's Quit.

use bind::Node;
use laserbeam::Ascend;

use crate::MercuryEffect;
use crate::state::MercuryPath;

/// Quit the program, whatever asked for it.
///
/// Generic over the event and the path, so home binds it to `q`'s `KeyEvent` from its own node and
/// the root binds it to the menu bar's `Quit`. The root binding is what makes the menu-bar
/// Quit a recovery path: it fires from any layer, unlike `q`, which quits only from home.
///
/// Emit the held modifiers' downs first. In a command layer their real downs were swallowed, so
/// the app does not know they are held; once the grab is released no further down is coming, so
/// tell it now, before `Kill`, or it is left thinking a physically-held modifier is up.
pub(crate) fn quit<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let mut effects = node.parent.ascend().typing_state.held.open();
    effects.push(MercuryEffect::Kill);
    effects
}
