//! The tab source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, TabEvent};

/// The browser reported the front tab's URL: record it on the foregrounded Chrome.
///
/// Dropped unless Chrome is the confirmed front app, which `set_tab_url` decides. A URL that
/// arrives while something else is up describes a window nobody is looking at, and one that
/// arrives mid-navigation belongs to the app being left. The site level rebuilds from this on
/// every dispatch, so there is nothing else to resync.
pub(crate) fn record_tab_url(ev: &TabEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.foreground.set_tab_url(ev.url.clone());
    Vec::new()
}
