# the clock is an event

What time it is belongs to the outside world, so it reaches the model the way every other outside fact does: a source observes it, sends an event, and a handler records it. A handler that wants to know the time reads a field that is already there.

`freddie_clock` is that source. It reports local civil time on every minute boundary, and `Mercury.clock` holds the last minute reported.

## Two clocks, and only one of them is this

The monotonic clock — `Instant`, and the duration between two of them — stays out of the model entirely. Timers own it: a handler asks for a delay, the effect loop sleeps, and the firing comes back as an event carrying which timer it was. No handler has ever seen an elapsed duration and none will; `event-timing.md` measures with the same clock, outside the model, for the same reason.

The wall clock is the other one: what a person would read off a watch. It is what a rule about the hour of the day is stated against, and it is what this makes an event.

## The tick is the minute

A rule stated on the wall clock is stated in minutes at the finest, so that is the resolution the model gets. The source truncates, so a tick's seconds and nanoseconds are always zero and two ticks in the same minute are equal.

It ticks whether or not anything is bound to it. A clock that runs only while something reads it needs the model to say who is reading, which is state about state, and the thing it would save is 1,440 dispatches a day at microseconds each.

That is also 1,440 records a day in the log file, which is the real cost and is worth knowing before the first time you read a day's log.

## The type

`jiff`, for the zone. Local civil time is the system zone applied to an instant, with the zone's DST rules, and the alternative is an event carrying a UTC instant and a handler doing that conversion — which puts tzdata inside `state.handle`.

The event carries `jiff::civil::DateTime`, which is `Copy`, `Eq`, `Ord`, and answers `weekday()`, `hour()`, and `date()` without another type. A test builds one:

```rust
jiff::civil::date(2026, 7, 21).at(9, 14, 0, 0)
```

`Cargo.toml`, in the workspace dependencies:

```toml
jiff = "0.2"
```

## Change 1: the crate

`crates/freddie_clock/src/lib.rs`. A `freddie_*` crate rather than mercury's, by the rule the README's crate list follows: figaro would write it identically.

Its shape is `freddie_windows`'s: `watch` hands back a watcher to hold and the seed to construct the model with, and dropping the watcher stops the ticks.

```rust
//! The wall clock as a source: one tick per minute, carrying local civil time.
//!
//! `watch` is the source and its return is the seed. There is no sink: nothing sets the time.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jiff::{Zoned, civil::DateTime};

/// The wall clock, truncated to the minute, in the system's zone.
#[must_use]
pub fn now() -> DateTime {
    let local = Zoned::now().datetime();
    local.with().second(0).subsec_nanosecond(0).build().unwrap_or(local)
}

/// How long until the next minute boundary.
///
/// Computed from the time it is asked, every tick, rather than by adding a minute to the last
/// deadline. A system clock that jumps — an ntp correction, a zone change, waking from sleep —
/// is then a tick that is at worst one minute late instead of a schedule that stays wrong.
#[must_use]
pub fn until_next_minute(now: SystemTime) -> Duration {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    Duration::from_secs(60 - secs % 60)
}

/// Ticks until dropped.
#[must_use = "dropping the watcher stops the clock; hold it to keep receiving ticks"]
pub struct Watcher {
    // Held only to be dropped: dropping it disconnects the thread's receiver, which is how the
    // thread learns to stop. Never sent on.
    #[expect(dead_code)]
    stop: Sender<()>,
}

/// Start ticking, and report the minute it is now.
///
/// `on_tick` runs on the clock's own thread and does one send, exactly as the other sources'
/// callbacks do.
pub fn watch(on_tick: impl Fn(DateTime) + Send + 'static) -> (Watcher, DateTime) {
    let (stop, stopped) = channel();
    std::thread::Builder::new()
        .name("freddie-clock".to_owned())
        .spawn(move || tick_until_stopped(&stopped, on_tick))
        .map_or_else(
            |e| tracing::error!(error = %e, "could not start the clock thread"),
            |_| (),
        );
    (Watcher { stop }, now())
}

/// Sleep to the next boundary, tick, repeat, until the watcher drops.
///
/// `recv_timeout` is the select: the timeout is the boundary and a disconnect is the watcher
/// going away, so the thread never wakes to ask whether it should still be running.
fn tick_until_stopped(stopped: &Receiver<()>, on_tick: impl Fn(DateTime)) {
    loop {
        match stopped.recv_timeout(until_next_minute(SystemTime::now())) {
            Err(RecvTimeoutError::Timeout) => on_tick(now()),
            Err(RecvTimeoutError::Disconnected) | Ok(()) => return,
        }
    }
}
```

The zone is whatever `Zoned::now()` says it is at the moment of the tick, so a machine that changes zone reports the new one from the next tick on. jiff owns that lookup and its caching.

The tests are the boundary arithmetic, which is the only thing here with an answer to get wrong:

```rust
#[test]
fn the_next_boundary_is_the_next_whole_minute() {
    for (epoch_secs, want) in [
        (0, 60),
        (1, 59),
        (59, 1),
        (60, 60),
        (61, 59),
        (1_753_072_499, 1),
    ] {
        let at = UNIX_EPOCH + Duration::from_secs(epoch_secs);
        assert_eq!(until_next_minute(at), Duration::from_secs(want), "{epoch_secs}");
    }
}
```

A boundary that lands exactly on the minute sleeps a whole minute rather than firing twice, which is what `60 - secs % 60` gives and what a `%` the other way around would not.

## Change 2: the trigger and the event

`crates/mercury/src/sources.rs`, beside the others:

```rust
/// A trigger that matches every clock tick.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Ticked;

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct ClockEvent {
    /// Local civil time, truncated to the minute.
    pub local: jiff::civil::DateTime,
}

impl EventTrigger for Ticked {
    type Event = ClockEvent;
    fn is_matching(&self, _ev: &ClockEvent) -> bool {
        true
    }
}
```

`crates/mercury/src/model.rs`, `MercuryTrigger`, before:

```rust
    Windowed(Windowed),
    Quit(Quit),
```

after:

```rust
    Windowed(Windowed),
    Ticked(Ticked),
    Quit(Quit),
```

and `MercuryEvent`, before:

```rust
    Window(WindowEvent),
    Quit(Quit),
```

after:

```rust
    Window(WindowEvent),
    Clock(ClockEvent),
    Quit(Quit),
```

with both re-exported from `crates/mercury/src/lib.rs` alongside `Windowed` and `WindowEvent`.

## Change 3: the state and the handler

`crates/mercury/src/state/mod.rs`, before:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub windows: Windows,
    pub typing_state: TypingState,
    overlay: Option<TimerGuard>,
    #[resolve_into]
    layer: Layer,
}
```

after:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub windows: Windows,
    pub typing_state: TypingState,
    /// The minute it is, as the clock source last reported it. Not an `Option`: the model is
    /// constructed with the minute it was constructed in, so there is no "we have not been told
    /// yet" for a handler to answer for.
    pub clock: DateTime,
    overlay: Option<TimerGuard>,
    #[resolve_into]
    layer: Layer,
}
```

with the bind at the root, before:

```rust
    Windowed => record_windows,
```

after:

```rust
    Windowed => record_windows,
    Ticked => record_clock,
```

`Mercury::new` takes it alongside the rest, and `crates/mercury/src/handlers/clock.rs` is the whole handler:

```rust
/// Record the minute. Assignment, so a tick applied twice lands where applying it once does.
pub(crate) fn record_clock(ev: &ClockEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    node.parent.clock = ev.local;
    Vec::new()
}
```

`crates/mercury/src/handlers/mod.rs` gains `mod clock;` and `pub(crate) use clock::*;`.

## Change 4: boot

`crates/mercury/src/daemon.rs`, `Boot`, before:

```rust
struct Boot {
    front_app: App,
    windows: Windows,
    window_sink: Option<WindowSink>,
}
```

after:

```rust
struct Boot {
    front_app: App,
    windows: Windows,
    window_sink: Option<WindowSink>,
    clock: DateTime,
}
```

and the watcher registered where the window watcher is, before the seed is read, by the same rule `seed-at-construction.md` states: every watcher is installed first, so a change in that window arrives twice and idempotence absorbs the duplicate. A minute that turns between `watch` returning its seed and the model being built sends a tick for a minute the model is already in, and the assignment lands on the same value.

```rust
let (_clock_watcher, clock) = freddie_clock::watch({
    let event_tx = event_tx.clone();
    move |local| {
        let _ = event_tx.send(MercuryEvent::Clock(ClockEvent { local }));
    }
});
```

The watcher is held for the life of the run, alongside `_window_watcher`. Unlike the window watcher it cannot fail: there is no permission to be denied and no observer to register, so there is no `Result` and no degraded mode.

## Change 5: the README

`crates/freddie_clock` joins the crate list:

```
- `freddie_clock`: the wall clock, one tick per minute.
```

## Tests

`crates/mercury/tests/transitions.rs`. The clock is a field the model assigns, so the table is what assignment owes:

```rust
fn minute(hour: i8, minute: i8) -> DateTime {
    jiff::civil::date(2026, 7, 21).at(hour, minute, 0, 0)
}

fn tick(at: DateTime) -> MercuryEvent {
    MercuryEvent::Clock(ClockEvent { local: at })
}

#[test]
fn a_tick_records_the_minute_and_asks_for_nothing() {
    let mut m = home();
    assert_eq!(m.handle(&tick(minute(9, 14))), Some(Vec::new()));
    assert_eq!(m.clock, minute(9, 14));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn a_tick_applied_twice_lands_where_once_does() {
    let mut m = home();
    let _ = m.handle(&tick(minute(9, 14)));
    let once = m.clock;
    let _ = m.handle(&tick(minute(9, 14)));
    assert_eq!(m.clock, once);
}

#[test]
fn a_tick_leaves_every_layer_where_it_found_it() {
    for mut m in [home(), nav(), resize(), typing(), in_app(App::Chrome)] {
        let before = m.layer_name();
        assert_eq!(m.handle(&tick(minute(9, 14))), Some(Vec::new()));
        assert_eq!(m.layer_name(), before);
    }
}
```

The last one is what stops a tick from ever becoming activity. A minute turning is not something the user did, so it does not reset a layer's return-home timer and does not take a passthrough layer out of passthrough.
