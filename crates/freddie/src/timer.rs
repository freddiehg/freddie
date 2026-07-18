//! An RAII timer.
//!
//! `timer_effect_and_guard` builds a linked pair: a [`DropGuard`](crate::DropGuard) the owning node
//! holds, and an event a handler returns as an effect. The effect loop reads the event's parts and
//! schedules them; dropping the guard cancels the timer.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::AlwaysEqual;
use crate::drop_guard::{DropGuard, drop_guard};

/// The scheduling half: a delay, the event to fire, and the cancel channel.
///
/// A handler returns it as an effect and the effect loop pattern-matches it to schedule. It owns
/// the event and the receiver, so it is used once. The receiver sits in `AlwaysEqual`, so the
/// effect's `testing` equality is the delay and the event; the effect, not the guard, carries the
/// testing concern.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerEffect<E> {
    pub delay: Duration,
    pub event: E,
    pub cancel: AlwaysEqual<oneshot::Receiver<()>>,
}

/// Build a linked guard/event pair that fires `event` after `delay`. The guard cancels the timer
/// on drop.
pub fn timer_effect_and_guard<E>(delay: Duration, event: E) -> (DropGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    (
        guard,
        TimerEffect {
            delay,
            event,
            cancel: AlwaysEqual(receiver),
        },
    )
}
