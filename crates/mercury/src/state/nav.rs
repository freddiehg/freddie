use bind::Bind;
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
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => show_overlay,
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
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
