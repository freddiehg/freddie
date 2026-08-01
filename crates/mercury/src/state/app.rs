use bind::{Bind, and, if_not_invalidated};
use freddie_keys::{Key, ModifierFlags};
use laserbeam::HasAncestor;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{App, MercuryStruct};

use super::{AppLayerPath, ForegroundedApp, MercuryPath, ReturnHomeLayersPath};

pub(crate) const CHROME_OVERLAY: &str = include_str!("overlays/chrome.txt");
pub(crate) const GHOSTTY_OVERLAY: &str = include_str!("overlays/ghostty.txt");
/// For an app with no bindings of its own: the in-app layer's own keys and nothing more.
pub(crate) const INAPP_OVERLAY: &str = include_str!("overlays/inapp.txt");

/// The keymap the overlay shows for the in-app layer while `app` is frontmost.
///
/// The in-app layer's bindings are the app's, so `i` in Ghostty and `i` in Chrome are different
/// keymaps and showing one for the other would be worse than showing nothing.
#[must_use]
pub(crate) const fn overlay_for(app: App) -> &'static str {
    match app {
        App::Chrome => CHROME_OVERLAY,
        App::Ghostty => GHOSTTY_OVERLAY,
        App::Finder | App::Zed | App::Other => INAPP_OVERLAY,
    }
}

/// The in-app layer. It stores NO app: `root.foreground` is the only copy, and [`app_data`]
/// builds the app's level from it on every dispatch. There is nothing to keep in sync and
/// nothing to go stale.
#[derive(Bind, Debug)]
#[node(parent_path = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::Escape.down() => if_not_invalidated(go_home),
    Key::KeyN.down() => if_not_invalidated(enter_nav),
    Key::KeyS.down() => if_not_invalidated(enter_site),
    Key::KeyT.down() => if_not_invalidated(enter_typing),
)]
pub struct AppLayer;

impl AppLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

/// The app's level, which is not in the tree. Several possible levels, so the data is an enum;
/// an app with no bindings is not a variant, and [`app_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent_path = AppLayerPath)]
#[binds(MercuryStruct)]
pub enum AppData {
    Chrome(ChromeApp),
    Ghostty(GhosttyApp),
}

/// Reads the confirmed front app, the only copy, and builds the level for it.
///
/// A shared reference, so it cannot mutate: it derives, it does not act. `None` while a nav is in
/// flight (the old app must not bind in the gap), and `Zed`/`Other` bind nothing, so all three get
/// no level and no struct.
fn app_data<'a, P: HasAncestor<MercuryPath<'a>>>(path: &P) -> Option<AppData> {
    let root = path.ancestor();
    match root.foreground.as_ref().map(ForegroundedApp::identity) {
        Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
        Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
        _ => None,
    }
}

/// Chrome's level. A unit: mercury tracks nothing per Chrome app. It stops being one when it
/// carries something (a tab name).
#[derive(Bind, Debug)]
#[derived_node(parent_path = AppLayerPath)]
#[binds(MercuryStruct)]
// `l` is bound at three modifier combinations, so all three are chords: a plain `KeyPress` ignores
// the flags, and any two of these would then match the same event.
#[bind(
    Key::KeyR.down() => if_not_invalidated(tap_cmd_r),
    Key::KeyL.down().bare() => if_not_invalidated(and!(tap_cmd_l, enter_typing)),
    Key::KeyL.down().with(ModifierFlags::SHIFT) => if_not_invalidated(copy_url),
    Key::KeyL.down().with(ModifierFlags::COMMAND) => if_not_invalidated(copy_host),
)]
pub struct ChromeApp;

impl ChromeApp {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

/// Ghostty's level, where `j` and `k` walk tmux's panes.
#[derive(Bind, Debug)]
#[derived_node(parent_path = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyJ.down() => if_not_invalidated(tmux_prev),
    Key::KeyK.down() => if_not_invalidated(tmux_next),
    Key::Num1.down() => if_not_invalidated(and!(tmux_window(Key::Num1), go_home)),
    Key::Num2.down() => if_not_invalidated(and!(tmux_window(Key::Num2), go_home)),
    Key::Num3.down() => if_not_invalidated(and!(tmux_window(Key::Num3), go_home)),
    Key::Num4.down() => if_not_invalidated(and!(tmux_window(Key::Num4), go_home)),
    Key::Num5.down() => if_not_invalidated(and!(tmux_window(Key::Num5), go_home)),
    Key::Num6.down() => if_not_invalidated(and!(tmux_window(Key::Num6), go_home)),
    Key::Num7.down() => if_not_invalidated(and!(tmux_window(Key::Num7), go_home)),
    Key::Num8.down() => if_not_invalidated(and!(tmux_window(Key::Num8), go_home)),
    Key::Num9.down() => if_not_invalidated(and!(tmux_window(Key::Num9), go_home)),
    Key::Num0.down() => if_not_invalidated(and!(tmux_window(Key::Num0), go_home)),
)]
pub struct GhosttyApp;

impl GhosttyApp {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
