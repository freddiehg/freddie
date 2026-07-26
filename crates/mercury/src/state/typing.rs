use std::time::Duration;

use bind::Bind;
use freddie::{KeySequence, TimerFired, TimerGuard, timer_effect_and_guard};
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{AnyKey, MercuryEffect, MercuryEvent, MercuryStruct};

use super::LayerPath;

/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/typing.txt");

/// The keys that leave typing for home.
const JK: &[Key] = &[Key::KeyJ, Key::KeyK];

/// How long a `jk` run waits for its next key before what it swallowed types itself.
///
/// It bounds how long a `j` stays invisible, so shorter is better, but it has to cover a
/// deliberately typed `jk` (down, up, down) rather than only a rolled one, which is far faster.
pub const JK_TIMEOUT: Duration = Duration::from_millis(200);

/// Arm a run's window: the guard cancels it on drop, the effect schedules it. The delay is the
/// run's own, read off the sequence, so this does not restate the policy.
///
/// `pub(crate)` because the handler that calls it is not a child of this module.
pub(crate) fn arm_jk_timeout(window: Duration) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(window, |id| MercuryEvent::Timer(TimerFired(id)));
    (guard, MercuryEffect::Timer(effect))
}

/// The typing layer. Its catch-all runs every key through the `jk` run and passes it through,
/// because typing is a passthrough layer. `jk` is the way out.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    // Only this run's window: a firing from a run that has since ended matches nothing, so the
    // handler never sees it.
    |path| path.get().jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    AnyKey => pass_through,
)]
pub struct TypingLayer {
    /// The `jk` run. Built fresh on entry and dropped with the layer, so a hold never outlives
    /// the layer it was typed in, and a pending window is cancelled by the leave that drops it.
    ///
    /// `pub` because the integration test crate reads it to assert where a run stands.
    pub jk: KeySequence,
}

impl TypingLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            jk: KeySequence::new(JK, Some(JK_TIMEOUT)),
        }
    }
}
