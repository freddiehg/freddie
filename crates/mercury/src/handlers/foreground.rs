//! The foreground source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{ForegroundEvent, MercuryEffect};

/// An app was foregrounded: record it at the root and end any pending navigation.
///
/// The in-app layer holds no app, so there is nothing to resync and nothing that can go stale:
/// `app_data` rebuilds the app's level from `root.foregrounded` on every dispatch. Layers other
/// than in-app are unaffected; foregrounding does not move you between them.
///
/// Clearing `has_navigated` is what turns the in-app level back on after a nav choice: while it
/// was set, `app_data` bound nothing, because `foregrounded` was still the old app; this event
/// is the watcher confirming the new front app, so the flag drops and the app's level resolves.
pub(crate) const fn on_foregrounded(
    ev: &ForegroundEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.foregrounded = ev.app;
    root.has_navigated = false;
    Vec::new()
}
