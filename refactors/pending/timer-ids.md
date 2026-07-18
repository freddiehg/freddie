# a timer that says which firing it is

A timer that has already fired cannot be un-fired. Dropping its guard cancels the sleep, but if the sleep finished a moment earlier its event is on the channel and the handler will see it. Every timer in mercury has this race:

- `LayerTimeout` firing late sends you home from a layer you had just entered.
- `JkTimeout` firing late interrupts a run that opened after the one it belonged to.
- The overlay's dwell firing late hides a showing that superseded it.

Today all three ignore it, because the windows are microseconds wide and the consequences are small. The fix is for a firing to say which arming it came from, and for the state that holds the guard to ignore one it is not waiting on.

The reason it has not been done is the bookkeeping that usually comes with it: a counter on the state, a mint-and-bump at each arm site, an id field on each event, and the discipline to keep them in step. That is a worse cost than the race.

So the id is minted by the timer, not by the caller. `timer_effect_and_guard` already builds the guard and the effect as a pair; it stamps both with the same id, and the caller never sees where the id comes from.

The whole cost at a call site is a closure at arm time and one comparison at handle time:

```rust
let (guard, effect) = timer_effect_and_guard(OVERLAY_DWELL, |id| {
    MercuryEvent::OverlayTimeout(OverlayTimeout(id))
});
self.overlay = Some(guard);
```

```rust
if self.overlay.as_ref().is_some_and(|guard| guard.armed(id)) {
    self.overlay = None;
    vec![MercuryEffect::HideOverlay]
} else {
    Vec::new()
}
```

There is no counter to keep, nothing to bump, and no state a caller can get wrong: a guard knows its own firing, and a timer that fires twice is not representable because the pair is built once.

A timer whose staleness does not matter keeps ignoring it. `arm_return_home` takes `|_id| MercuryEvent::LayerTimeout(LayerTimeout)` and nothing changes for it; adopting the check later is a one-line change at the handler, not a redesign.

## change 1: the id, minted with the pair

`crates/freddie/src/drop_guard.rs`, the guard carries the id it was armed with:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// Identifies one arming of one timer.
///
/// Minted by [`timer_effect_and_guard`](crate::timer_effect_and_guard) and stamped on both halves,
/// so a fired event can be matched against the guard still held. Process-wide and monotonic: two
/// armings never share one, whoever armed them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId(u64);

impl TimerId {
    /// The next id. Relaxed: the only requirement is that no two calls return the same value.
    pub(crate) fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
```

`DropGuard` before:

```rust
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard(
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[expect(dead_code)] oneshot::Sender<()>,
);
```

after:

```rust
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard {
    id: TimerId,
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[expect(dead_code)]
    cancel: oneshot::Sender<()>,
}

impl DropGuard {
    /// Whether `id` is the firing this guard is waiting on.
    ///
    /// A guard that was replaced has already been dropped, so asking this of the guard a state
    /// still holds answers "is this event still mine, or did it come from an arming I abandoned".
    #[must_use]
    pub fn armed(&self, id: TimerId) -> bool {
        self.id == id
    }
}
```

`drop_guard` mints the id and hands it back, so the timer can stamp the event with it:

```rust
/// Build a linked guard/receiver pair, and the id identifying this arming.
pub fn drop_guard() -> (DropGuard, oneshot::Receiver<()>, TimerId) {
    let (cancel, receiver) = oneshot::channel();
    let id = TimerId::mint();
    (DropGuard { id, cancel }, receiver, id)
}
```

## change 2: the timer stamps the event

`crates/freddie/src/timer.rs`, before:

```rust
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
```

after:

```rust
/// Build a linked guard/event pair that fires after `delay`. The guard cancels the timer on drop.
///
/// `event` is handed the id identifying this arming, so the event it builds can carry it. A
/// handler that cares compares it against the guard its state still holds (see
/// [`DropGuard::armed`]); one that does not takes `|_id| ..` and ignores it.
pub fn timer_effect_and_guard<E>(
    delay: Duration,
    event: impl FnOnce(TimerId) -> E,
) -> (DropGuard, TimerEffect<E>) {
    let (guard, receiver, id) = drop_guard();
    (
        guard,
        TimerEffect {
            delay,
            event: event(id),
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

`crates/freddie/src/lib.rs` adds `TimerId` to the `drop_guard` re-export.

## change 3: mercury's arm sites

`crates/mercury/src/state/mod.rs`, both helpers wrap their event in a closure. `arm_return_home`, before:

```rust
    let (guard, effect) = timer_effect_and_guard(
        RETURN_TO_HOME_TIMEOUT,
        MercuryEvent::LayerTimeout(LayerTimeout),
    );
```

after:

```rust
    // The id is ignored: a stale return-home lands in whatever layer is active and sends it home,
    // which is where a stale one would have sent it anyway.
    let (guard, effect) = timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, |_id| {
        MercuryEvent::LayerTimeout(LayerTimeout)
    });
```

`arm_jk_timeout` changes the same way, with `|_id| MercuryEvent::JkTimeout(JkTimeout)`.

`crates/mercury/tests/transitions.rs` rebuilds two timer effects to assert against (`return_home_timer` and `jk_timer`); both take the closure form. Their `testing` equality is the delay and the event, and the event no longer varies, so the assertions are unchanged.

## change 4: the tests

`crates/freddie/tests/`, new, over the primitive:

```rust
#[test]
fn a_guard_knows_its_own_arming() {
    let (guard, _rx) = timer_effect_and_guard(Duration::from_secs(1), |id| id);
    // The effect carries the id the guard was armed with.
    assert!(guard.armed(effect.event));
}

#[test]
fn two_armings_never_share_an_id() {
    let (first, _) = timer_effect_and_guard(Duration::from_secs(1), |id| id);
    let (second, other) = timer_effect_and_guard(Duration::from_secs(1), |id| id);
    assert!(!first.armed(other.event), "the second arming's id is not the first's");
    assert!(second.armed(other.event));
}
```

## what this does not do

It does not unify the three timer event types into one. Each timer still has its own trigger and event; this only gives every arming an identity. Folding `LayerTimeout`, `JkTimeout`, and `OverlayTimeout` into one `TimerFired` event is a separate question, and the answer changes if a timer ever needs to carry more than its identity.
