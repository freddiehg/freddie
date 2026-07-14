//! The foreground source's one handler.

use bind::Node;

use crate::tree::Mercury;
use crate::{ForegroundEvent, MercuryEffect};

/// An app was foregrounded: record it at the root.
///
/// The in-app layer holds no app, so there is nothing to resync and nothing that can go stale:
/// `app_data` rebuilds the app's level from `root.foregrounded` on every dispatch. Layers other
/// than in-app are unaffected; foregrounding does not move you between them.
pub(crate) const fn on_foregrounded(
    ev: &ForegroundEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.foregrounded = ev.app;
    Vec::new()
}
