//! The window source's one handler.

use bind::AscendState;
use freddie::TimerFired;
use laserbeam::{Complete, Completed};

use crate::state::MercuryPath;
use crate::{MercuryEffect, WindowEvent};

/// The windows changed: record it at the root.
///
/// Nothing else happens on a window event. Placements read [`Windows`](crate::state::Windows)
/// when a key asks for one; the source's job is only to keep it true.
pub(crate) fn record_windows<'x>(
    ev: &WindowEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    root.windows.record(&ev.change);
    (Vec::new(), root.complete())
}

/// The placement mercury asked for has had its time: whatever the window has done since,
/// what it does next is the user's.
pub(crate) fn placement_settled<'x>(
    _ev: &TimerFired,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    root.windows.forget_pending();
    (Vec::new(), root.complete())
}
