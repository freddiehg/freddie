//! The root's `AnyKey` post: tracking the held modifiers.

use bind::AscendState;
use freddie_keys::KeyEvent;
use laserbeam::{Complete, Completed};

use crate::MercuryEffect;
use crate::state::MercuryPath;

/// Every key, claimed or not: keep `held` true.
///
/// `held` feeds the open and close sweeps a layer change runs, so it has to see a modifier
/// pressed in a command layer, where a deeper binding may have claimed the key. That is what
/// makes this a post: it is scheduled by the trigger alone and takes no claim.
///
/// The flags on the event stay authoritative for what a key carries; `held` is for the sweeps.
pub(crate) fn track_held_modifiers<'x>(
    ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    if ev.key.is_modifier() {
        root.held.apply(ev);
    }
    (vec![], root.complete())
}
