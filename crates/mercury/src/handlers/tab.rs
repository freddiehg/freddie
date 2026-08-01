//! The tab source's one handler.

use laserbeam::{Completed, CompletesTo};

use crate::state::{ForegroundedApp, MercuryPath};
use crate::{MercuryEffect, TabEvent};

/// The browser reported the front tab's URL: record it on the foregrounded Chrome.
///
/// Dropped unless Chrome is the confirmed front app, which `set_tab_url` decides. A URL that
/// arrives while something else is up describes a window nobody is looking at, and one that
/// arrives mid-navigation belongs to the app being left. The site level rebuilds from this on
/// every dispatch, so there is nothing else to resync.
pub(crate) fn record_tab_url<'x>(
    ev: &TabEvent,
    _snap: (),
    p: MercuryPath<'x>,
) -> (Vec<MercuryEffect>, Completed<MercuryPath<'x>>) {
    let root: MercuryPath<'x> = p;
    if let Some(ForegroundedApp::Chrome(chrome)) = &mut root.foreground {
        chrome.url = Some(ev.url.clone());
    }
    (Vec::new(), root.complete())
}
