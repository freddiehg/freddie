use bind::{Bind, and};
use freddie::TimerGuard;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{MercuryEffect, MercuryStruct};

use super::{LayerPath, arm_return_home};

/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/nav.txt");

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    // Only this layer's own timer: a firing from a layer already left matches nothing.
    |path| path.get().home_timeout.trigger() => go_home,
    Key::Escape.down() => go_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => enter_typing,
    Key::KeyC.down() => and!(mark_navigating, foreground_chrome, enter_inapp),
    Key::KeyF.down() => and!(mark_navigating, foreground_finder, enter_inapp),
    Key::KeyG.down() => and!(mark_navigating, foreground_ghostty, enter_inapp),
    Key::KeyZ.down() => and!(mark_navigating, foreground_zed, enter_inapp),
    Key::Space.down() => and!(tap_cmd_space, enter_typing),
)]
pub struct NavLayer {
    // Read for the trigger matching its firing, and held for its `Drop`: dropping the guard cancels nav's return-home timer.
    pub(crate) home_timeout: TimerGuard,
}

impl NavLayer {
    /// Build the nav layer with its return-home timer armed, returning the layer and the effect
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
}
