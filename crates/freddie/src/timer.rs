//! An RAII timer.
//!
//! `timer_effect_and_guard` builds a linked pair: a [`DropGuard`](crate::DropGuard) the owning node
//! holds, and an event a handler returns as an effect. The effect loop reads the event's parts and
//! schedules them; dropping the guard cancels the timer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bind::EventTrigger;
use tokio::sync::oneshot;

use crate::AlwaysEqual;
use crate::drop_guard::{DropGuard, drop_guard};

/// Identifies one timer.
///
/// Minted when the timer is set and stamped on both halves, so a fired event can be matched
/// against the guard still held. Process-wide and monotonic: two timers never share one, whoever
/// set them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId(u64);

impl TimerId {
    /// The next id.
    ///
    /// Atomic because a mutable static has to be `Sync`, not because anything sets timers off one
    /// thread; `Relaxed` because the only requirement is that no two calls return the same value.
    fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// The guard for one timer: what cancels it, and which timer it is.
///
/// Dropping it cancels the timer, because it drops the [`DropGuard`] inside it. Keeping it is what
/// lets a binding match the event this timer will fire and no other.
#[must_use = "dropping the guard cancels the timer immediately"]
#[derive(Debug)]
pub struct TimerGuard {
    id: TimerId,
    // Held only to be dropped: dropping it wakes the receiver the effect carries. Never read.
    #[expect(dead_code)]
    guard: DropGuard,
}

impl TimerGuard {
    /// The trigger matching this timer's own firing, and no other.
    #[must_use]
    pub const fn trigger(&self) -> TimerTrigger {
        TimerTrigger(self.id)
    }
}

/// A timer fired, carrying which timer it was.
///
/// One event for every timer a consumer owns. What tells them apart at dispatch is which node is
/// still holding that guard, not which type the event is.
#[derive(Debug)]
pub struct TimerFired(pub TimerId);

/// Two firings compare equal under `testing` whatever their ids.
///
/// The id exists to tell one timer from another at dispatch. A test that rebuilds an expected
/// effect cannot know it, and asserting it would only assert that the counter ran; with one event
/// type for every timer, the delay is what distinguishes an effect anyway. A test that cares about
/// an id reads it off the effect a transition produced.
#[cfg(feature = "testing")]
impl PartialEq for TimerFired {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(feature = "testing")]
impl Eq for TimerFired {}

/// Matches only the firing of the timer it was built from.
///
/// Its value comes from the guard the bound node holds, so a binding written with it fires for its
/// own timer and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerTrigger(TimerId);

impl EventTrigger for TimerTrigger {
    type Event = TimerFired;
    fn is_matching(&self, ev: &TimerFired) -> bool {
        self.0 == ev.0
    }
}

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

/// Build a linked guard and event that fires after `delay`. Dropping the guard cancels the timer.
///
/// `event` is handed the id identifying this timer, so the event it builds carries it and the
/// binding that wants it can match on the guard still held.
pub fn timer_effect_and_guard<E>(
    delay: Duration,
    event: impl FnOnce(TimerId) -> E,
) -> (TimerGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    let id = TimerId::mint();
    (
        TimerGuard { id, guard },
        TimerEffect {
            delay,
            event: event(id),
            cancel: AlwaysEqual(receiver),
        },
    )
}
