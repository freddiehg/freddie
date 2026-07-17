# timers

A timer is a source: it fires an event, that event dispatches like a keypress, and a handler runs. It comes as two linked halves. `timer_effect_and_guard::<E>(delay, event)` returns `(TimerGuard, TimerEffect<E>)`: the owning node holds the guard, the handler returns the event as an effect, the effect loop schedules it, and dropping the guard cancels the timer. Clobbering a timer is dropping the guard and building a new pair; leaving a state drops the node and its guard.

This doc is the timer primitive and the effect that carries it. The consumers are `refactors/pending/layer-timeout.md` and `refactors/pending/overlay.md`.

## change 1: the freddie timer primitive

`crates/freddie/Cargo.toml`, before:

```toml
[features]
# Test-only equality: `AlwaysEqual` compares equal only under this, so it stays out of the
# normal build.
testing = []
```

after:

```toml
[dependencies]
tokio = { version = "1", features = ["sync"] }

[features]
# Test-only equality: `AlwaysEqual` compares equal only under this, so it stays out of the
# normal build.
testing = []
```

`crates/freddie/src/lib.rs`, before:

```rust
//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;

pub use always_equal::AlwaysEqual;
```

after:

```rust
//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;
pub mod timer;

pub use always_equal::AlwaysEqual;
pub use timer::{timer_effect_and_guard, TimerEffect, TimerGuard};
```

New `crates/freddie/src/timer.rs`:

```rust
//! An RAII timer. `timer_effect_and_guard` builds a linked pair: a guard the owning node holds,
//! and an event a handler returns as an effect. The effect loop reads the event's parts and
//! schedules them; dropping the guard cancels the timer.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::AlwaysEqual;

/// The cancelling half, held by the node that owns the timer. Dropping it (a transition that
/// replaces the node, or a clobber that overwrites the guard) cancels the timer at once, because
/// the paired receiver wakes when this sender goes.
#[must_use = "dropping the guard cancels the timer immediately"]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerGuard(AlwaysEqual<oneshot::Sender<()>>);

/// The scheduling half: a delay, the event to fire, and the cancel channel. A handler returns it
/// as an effect and the effect loop pattern-matches it to schedule. It owns the event and the
/// receiver, so it is used once. The receiver sits in `AlwaysEqual`, so the effect's `testing`
/// equality is the delay and the event.
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
```

Both `TimerGuard` and `TimerEffect` derive equality under `testing` (the guard so a state node holding one stays assertable, the event so the effect does), and the `AlwaysEqual` fields keep the channel handles out of that equality.

## change 2: the Timer effect, and an effect loop that owns the effect

`crates/mercury/Cargo.toml`, `[dependencies]`, add:

```toml
freddie = { path = "../freddie", version = "0.0.1" }
```

`crates/mercury/Cargo.toml`, the `testing` feature, before:

```toml
testing = []
```

after (mercury's `testing` turns freddie's on, so `TimerEffect`'s equality is present):

```toml
testing = ["freddie/testing"]
```

`crates/mercury/src/effect.rs` gains `use freddie::TimerEffect;` and `use crate::MercuryEvent;`, and `MercuryEffect` gains the variant. Before:

```rust
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
    /// Quit the program. The effect handler performs this by exiting.
    Kill,
}
```

after:

```rust
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
    /// Quit the program. The effect handler performs this by exiting.
    Kill,
    /// Arm a timer. The effect loop schedules it; it fires its event after the delay unless the
    /// guard held by the state that asked for it drops first.
    Timer(TimerEffect<MercuryEvent>),
}
```

`crates/mercury/src/main.rs`. `perform_effect` takes the effect by value, so a timer's receiver can move into the spawned task, and the effect loop hands over the event sender.

`run`'s `select!`, before:

```rust
    tokio::select! {
        () = run_event_loop(mercury, event_rx, effect_tx) => {}
        () = run_effect_loop(effect_rx, emitter) => {}
    }
```

after (`event_tx` is still owned here, after its clones for the sources):

```rust
    tokio::select! {
        () = run_event_loop(mercury, event_rx, effect_tx) => {}
        () = run_effect_loop(effect_rx, emitter, event_tx) => {}
    }
```

`run_effect_loop`, before:

```rust
#[allow(clippy::future_not_send)]
async fn run_effect_loop(mut effect_rx: UnboundedReceiver<MercuryEffect>, emitter: Emitter) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(&effect, &emitter).is_break() {
            break;
        }
    }
}
```

after:

```rust
#[allow(clippy::future_not_send)]
async fn run_effect_loop(
    mut effect_rx: UnboundedReceiver<MercuryEffect>,
    emitter: Emitter,
    event_tx: UnboundedSender<MercuryEvent>,
) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(effect, &emitter, &event_tx).is_break() {
            break;
        }
    }
}
```

`perform_effect`, before:

```rust
fn perform_effect(effect: &MercuryEffect, emitter: &Emitter) -> ControlFlow<()> {
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(*app),
        MercuryEffect::Tap { key, flags } => match emitter.tap(*key, *flags) {
            Ok(()) => debug!(?key, ?flags, "tapped"),
            Err(e) => warn!(?key, ?flags, error = %e, "tap failed"),
        },
        MercuryEffect::Emit(ke) => match emitter.emit(ke.key, ke.press, ke.flags) {
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
            Err(e) => warn!(key = ?ke.key, press = ?ke.press, error = %e, "emit failed"),
        },
        MercuryEffect::Place(placement) => place_window(*placement),
        MercuryEffect::Kill => {
            info!("kill: exiting");
            return ControlFlow::Break(());
        }
    }
    ControlFlow::Continue(())
}
```

after (owned `effect`, so the copies drop their `*`; the new arm hands the timer to a `schedule_timer` performer, beside `foreground_app` and `place_window`):

```rust
fn perform_effect(
    effect: MercuryEffect,
    emitter: &Emitter,
    event_tx: &UnboundedSender<MercuryEvent>,
) -> ControlFlow<()> {
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(app),
        MercuryEffect::Tap { key, flags } => match emitter.tap(key, flags) {
            Ok(()) => debug!(?key, ?flags, "tapped"),
            Err(e) => warn!(?key, ?flags, error = %e, "tap failed"),
        },
        MercuryEffect::Emit(ke) => match emitter.emit(ke.key, ke.press, ke.flags) {
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
            Err(e) => warn!(key = ?ke.key, press = ?ke.press, error = %e, "emit failed"),
        },
        MercuryEffect::Place(placement) => place_window(placement),
        MercuryEffect::Kill => {
            info!("kill: exiting");
            return ControlFlow::Break(());
        }
        MercuryEffect::Timer(timer) => schedule_timer(timer, event_tx),
    }
    ControlFlow::Continue(())
}
```

`schedule_timer` is the new performer, detaching its work the way the others do and pattern-matching the timer:

```rust
/// Schedule `timer`: fire its event after its delay, unless the guard the model kept drops first.
/// Fire-and-forget on the runtime, like `foreground_app` and `place_window`, so a pending sleep
/// runs off the effect loop.
fn schedule_timer(timer: TimerEffect<MercuryEvent>, event_tx: &UnboundedSender<MercuryEvent>) {
    let TimerEffect {
        delay,
        event,
        cancel: AlwaysEqual(mut cancel),
    } = timer;
    // If the guard dropped before the loop reached this, the receiver is closed and the timer is
    // already cancelled, so there is nothing to spawn.
    if matches!(cancel.try_recv(), Err(TryRecvError::Closed)) {
        return;
    }
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep(delay) => { let _ = event_tx.send(event); }
            _ = cancel => {}
        }
    });
}
```

`main.rs` gains `use freddie::{AlwaysEqual, TimerEffect};` and `use tokio::sync::oneshot::error::TryRecvError;`, and already imports `UnboundedSender` and `MercuryEvent`.
