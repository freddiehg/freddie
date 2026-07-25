//! Showing and hiding the overlay: `o` in a layer that binds keys toggles its keymap, and the
//! hide timer takes it down on its own.

use bind::AscendState;
use freddie::TimerFired;
use laserbeam::{Complete, Completed, HasStop, IntoAncestor, MaybeInvalidated};

use crate::MercuryEffect;
use crate::state::MercuryPath;

/// `o` in a layer that binds keys: show that layer's keymap, or take it down if it is up.
///
/// Generic over the event and the path, so every such layer binds it from its own node.
pub(crate) fn toggle_overlay<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
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
