use std::time::Duration;

use bind::Bind;
use freddie::{TimerGuard, timer_effect_and_guard};
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{LayerTimeout, MercuryEffect, MercuryEvent, MercuryStruct};

use super::LayerPath;

/// How long nav sits idle before returning home.
pub const RETURN_TO_HOME_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {
    // Held for its `Drop`: dropping the guard cancels nav's return-home timer.
    #[allow(dead_code)]
    timeout: TimerGuard,
}

impl NavLayer {
    /// Build the nav layer with its idle-timeout armed, returning the layer and the effect that
    /// schedules the timeout.
    #[must_use]
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, effect) = timer_effect_and_guard(
            RETURN_TO_HOME_TIMEOUT,
            MercuryEvent::LayerTimeout(LayerTimeout),
        );
        (Self { timeout }, MercuryEffect::Timer(effect))
    }
}
