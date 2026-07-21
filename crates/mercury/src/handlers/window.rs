//! The window source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, WindowEvent};

/// The windows changed: record it at the root.
///
/// Nothing else happens on a window event. Placements read [`Windows`](crate::state::Windows)
/// when a key asks for one; the source's job is only to keep it true.
pub(crate) fn record_windows(ev: &WindowEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.windows.record(&ev.change);
    Vec::new()
}
