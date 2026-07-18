use bind::Bind;
use freddie::TimerGuard;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{App, MercuryEffect, MercuryStruct};

use super::{AppLayerPath, LayerPath, arm_return_home};

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
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    // Only this layer's own timer: a firing from a layer already left matches nothing.
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
)]
pub struct AppLayer {
    // Read for the trigger matching its firing, and held for its `Drop`: dropping the guard cancels the in-app layer's return-home timer.
    pub(crate) home_timeout: TimerGuard,
}

impl AppLayer {
    /// Build the in-app layer with its return-home timer armed, returning the layer and the effect
    /// that schedules it.
    #[must_use]
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home();
        (
            Self {
                home_timeout: timeout,
            },
            timer,
        )
    }

    /// Reset the return-home timer on in-app activity: drop the old guard (cancelling it) and arm a
    /// fresh one, returning the effect that schedules it.
    #[must_use]
    pub(crate) fn rearm(&mut self) -> MercuryEffect {
        let (timeout, timer) = arm_return_home();
        self.home_timeout = timeout;
        timer
    }
}

/// The app's level, which is not in the tree. Several possible levels, so the data is an enum;
/// an app with no bindings is not a variant, and [`app_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
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
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    // AppLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    match root.foreground.confirmed() {
        Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
        Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
        _ => None,
    }
}

/// Chrome's level. A unit for now: mercury tracks nothing per app. It stops being one when it
/// carries something (a tab name).
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyR.down() => refresh)]
pub struct ChromeApp {}

impl ChromeApp {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}

/// Ghostty's level, where `j` and `k` walk tmux's panes.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyJ.down() => previous_window,
    Key::KeyK.down() => next_window,
    Key::Num1.down() => window_1,
    Key::Num2.down() => window_2,
    Key::Num3.down() => window_3,
    Key::Num4.down() => window_4,
    Key::Num5.down() => window_5,
    Key::Num6.down() => window_6,
    Key::Num7.down() => window_7,
    Key::Num8.down() => window_8,
    Key::Num9.down() => window_9,
    Key::Num0.down() => window_0,
)]
pub struct GhosttyApp {}

impl GhosttyApp {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}
