//! Showing and hiding the overlay: `o` toggles the active layer's keymap, and the hide timer
//! takes it down on its own.

use bind::AscendState;
use freddie::TimerFired;
use freddie_keys::KeyEvent;
use laserbeam::{Completed, CompletesTo};

use crate::MercuryEffect;
use crate::state::MercuryPath;

/// `o`: show the active layer's keymap, or take it down if it is up.
///
/// Bound at the root, whose own field `overlay` is, so this is an own-node write: it reads the
/// layer beneath for the content ([`Mercury::toggle_overlay`](crate::state::Mercury) does that)
/// and hands back its own completion, invalidating nothing below. The deadline post ran earlier,
/// during the ascent beneath it, and read a true stay, so pressing `o` counts as activity.
///
/// Which layers answer to `o` is the trigger's business, not this handler's: typing has no `o`
/// trigger, because there an `o` is an `o`.
pub(crate) fn toggle_overlay<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    let effects = root.toggle_overlay();
    (effects, root.complete())
}

/// The overlay's hide timer fired. Bound at the root, so it fires from whatever layer is active,
/// and only for the showing still up: the binding matches the guard the root holds.
pub(crate) fn hide_overlay<'x>(
    _ev: &TimerFired,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    (root.hide_overlay(), root.complete())
}
