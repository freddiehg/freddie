use bind::Bind;
use freddie::TimerGuard;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{MercuryEffect, MercuryStruct};

use super::{LayerPath, arm_return_home};

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser, so it idles back home too.
/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/resize.txt");

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    // Only this layer's own timer: a firing from a layer already left matches nothing.
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => to_typing,
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
)]
pub struct ResizeLayer {
    // Read for the trigger matching its firing, and held for its `Drop`: dropping the guard cancels resize's return-home timer.
    pub(crate) home_timeout: TimerGuard,
}

impl ResizeLayer {
    /// Build the resize layer with its return-home timer armed, returning the layer and the effect
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
