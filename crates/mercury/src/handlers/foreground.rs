//! The foreground source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{ForegroundEvent, MercuryEffect};

/// An app was foregrounded: record it at the root and end any pending navigation.
///
/// The in-app layer holds no app, so there is nothing to resync and nothing that can go stale:
/// `app_data` rebuilds the app's level from `root.foreground` on every dispatch. Layers other
/// than in-app are unaffected; foregrounding does not move you between them. This is the watcher
/// confirming the new front app, so it also ends a pending nav and the app's level resolves again.
pub(crate) const fn on_foregrounded(
    ev: &ForegroundEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.foreground.on_foregrounded_app_event(ev.app);
    Vec::new()
}
