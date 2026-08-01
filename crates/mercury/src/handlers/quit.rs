//! The program's one way out, shared by home's `q` and the menu bar's Quit.

use bind::AscendState;
use laserbeam::{Completed, CompletesTo, HasStop, IntoAncestor, MaybeInvalidated};

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
pub(crate) fn quit<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let mut effects = root.held.open();
    effects.push(MercuryEffect::Kill);
    (effects, root.complete())
}
