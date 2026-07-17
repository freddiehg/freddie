//! An RAII timer.
//!
//! `timer_effect_and_guard` builds a linked pair: a guard the owning node holds, and an event a
//! handler returns as an effect. The effect loop reads the event's parts and schedules them;
//! dropping the guard cancels the timer.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::AlwaysEqual;

/// The cancelling half, held by the node that owns the timer.
///
/// Dropping it (a transition that replaces the node, or a clobber that overwrites the guard)
/// cancels the timer at once, because the paired receiver wakes when this sender goes.
#[must_use = "dropping the guard cancels the timer immediately"]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerGuard(
    // Held for its `Drop`: dropping the sender wakes the paired receiver and cancels the timer. It
    // is read only under `testing` (the equality derive), so it is otherwise never read.
    #[cfg_attr(not(feature = "testing"), allow(dead_code))] AlwaysEqual<oneshot::Sender<()>>,
);

/// The scheduling half: a delay, the event to fire, and the cancel channel.
///
/// A handler returns it as an effect and the effect loop pattern-matches it to schedule. It owns
/// the event and the receiver, so it is used once. The receiver sits in `AlwaysEqual`, so the
/// effect's `testing` equality is the delay and the event.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerEffect<E> {
    pub delay: Duration,
    pub event: E,
    pub cancel: AlwaysEqual<oneshot::Receiver<()>>,
}

/// Build a linked guard/event pair that fires `event` after `delay`.
pub fn timer_effect_and_guard<E>(delay: Duration, event: E) -> (TimerGuard, TimerEffect<E>) {
    let (sender, cancel) = oneshot::channel();
    (
        TimerGuard(AlwaysEqual(sender)),
        TimerEffect {
            delay,
            event,
            cancel: AlwaysEqual(cancel),
        },
    )
}
