//! The foreground source's one handler.

use bind::AscendState;
use laserbeam::{Completed, CompletesTo};

use crate::state::MercuryPath;
use crate::{ForegroundEvent, MercuryEffect};

/// An app was foregrounded: record it at the root and end any pending navigation.
///
/// The in-app layer holds no app, so there is nothing to resync and nothing that can go stale:
/// `app_data` rebuilds the app's level from `root.foreground` on every dispatch. Layers other
/// than in-app are unaffected; foregrounding does not move you between them. This is the watcher
/// confirming the new front app, so it also ends a pending nav and the app's level resolves again.
pub(crate) fn record_front_app<'x>(
    ev: &ForegroundEvent,
    _snap: (),
    st: AscendState<'_, MercuryPath<'x>>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = st.state.into_ancestor();
    root.foreground.set_front_app(ev.app);
    (Vec::new(), root.complete())
}
