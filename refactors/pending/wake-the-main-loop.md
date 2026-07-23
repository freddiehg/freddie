# waking the main loop on send

`MainLoop::run` parks the main thread in `nextEventMatchingMask_untilDate_inMode_dequeue` with a `SLICE` (100ms) deadline, and runs `on_wake` each time that returns — on an `AppKit` event or the deadline. Work handed to the main thread through a channel drained in `on_wake` therefore waits until the next return. A plain channel send neither posts an `NSEvent` nor breaks the wait, so on a quiet loop the value sits up to `SLICE`.

The menu-bar title already pays this. Its channel is drained in `on_wake`, and a layer change driven by a swallowed key posts no `NSEvent`, so the title can update up to 100ms after the keystroke. Anything else that marshals to the main thread this way — the overlay, once it stops using GCD — pays the same.

This change adds a handle that breaks the wait so `on_wake` runs at once, and a channel sender that wakes on every send, so a consumer cannot forget to. The title moves to it here; the overlay change is a later consumer.

## The wake mechanism

`wake` has to make a blocked `nextEventMatchingMask` return. `CFRunLoop::stop` on the main run loop, called from another thread, is the candidate: it is already how `Stopper` breaks the loop, `CFRunLoop` is `Send`, and `get_main` works off the main thread. The loop returns from `nextEventMatchingMask`, runs `on_wake`, checks its stop channel (unchanged, still open), and turns again. A `wake` and a shutdown both stop the run loop; the loop tells them apart by the stop channel, not by the stop call.

### Verify before implementing

`Stopper` has never proven `CFRunLoop::stop` breaks `nextEventMatchingMask`, because it falls back on the `SLICE`: a shutdown that takes 100ms instead of being instant is invisible, so the poke could be a no-op and no test would catch it. Wake-on-send has no such fallback — the whole point is promptness — so this must be confirmed with a spike (in a scratch binary, not the repo): init `NSApplication` as accessory on the main thread, park it in the same `nextEventMatchingMask` loop with a multi-second `SLICE`, and from a spawned thread call `CFRunLoop::get_main().stop()` after a delay. If `on_wake` fires at the delay rather than at the `SLICE`, `stop` is the mechanism and the design below stands.

If it does not, `wake` instead posts a dummy event, which `nextEventMatchingMask` dequeues and returns:

```rust
// fallback wake: post an application-defined NSEvent to the front of the queue.
// `postEvent:atStart:` is callable off the main thread. The event carries no meaning; `sendEvent`
// ignores an application-defined event with no handler, and the point is only that the dequeue
// returns so the loop runs `on_wake`.
```

The rest of this doc assumes `stop` works. If the fallback is needed, `MainWaker` holds what it needs to post the event instead of a `CFRunLoop`, and only `MainWaker::wake` changes.

## Change 1: `MainWaker` and `WakingSender`

`crates/freddie_main_loop/src/lib.rs`, added below `Stopper`:

```rust
/// Wakes the main loop from any thread, so work queued for `on_wake` runs now rather than at the
/// next `SLICE`. Cheap to clone; give one to each thread that sends on a channel `on_wake` drains.
#[derive(Clone)]
pub struct MainWaker {
    run_loop: CFRunLoop,
}

impl Default for MainWaker {
    fn default() -> Self {
        Self::new()
    }
}

impl MainWaker {
    /// A waker for this process's main run loop. Callable from any thread; `get_main` returns the
    /// main run loop wherever it is called.
    #[must_use]
    pub fn new() -> Self {
        Self {
            run_loop: CFRunLoop::get_main(),
        }
    }

    /// Break the main loop's current wait so it turns and runs `on_wake` at once.
    ///
    /// Send on the channel first, then call this: the send has to be visible when `on_wake` drains,
    /// the same ordering `Stopper` uses for its flag and stop.
    pub fn wake(&self) {
        self.run_loop.stop();
    }

    /// A channel whose sender wakes the loop on every send. The receiver is a plain
    /// `std::sync::mpsc::Receiver`, drained in `on_wake` on the main thread.
    #[must_use]
    pub fn channel<T>(&self) -> (WakingSender<T>, Receiver<T>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            WakingSender {
                tx,
                waker: self.clone(),
            },
            rx,
        )
    }
}

/// A channel sender that wakes the main loop after each send, so the value reaches `on_wake` now
/// instead of at the next `SLICE`. `Send` and `Clone`, for the worker threads that send.
///
/// Waking on send is the point: a consumer holds one of these instead of a bare `Sender`, so it
/// cannot send without waking. That is enforced here rather than left to each send site to remember.
pub struct WakingSender<T> {
    tx: Sender<T>,
    waker: MainWaker,
}

impl<T> Clone for WakingSender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<T> WakingSender<T> {
    /// Send, then wake the main loop.
    ///
    /// # Errors
    ///
    /// If the receiver has been dropped, the same as [`std::sync::mpsc::Sender::send`].
    pub fn send(&self, value: T) -> Result<(), std::sync::mpsc::SendError<T>> {
        self.tx.send(value)?;
        self.waker.wake();
        Ok(())
    }
}
```

`Sender` and `Receiver` come from a widened import; `crates/freddie_main_loop/src/lib.rs`, before:

```rust
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
```

after:

```rust
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
```

is unchanged — `Receiver` and `Sender` are already imported for the stop channel.

`MainWaker` is `Send` because `CFRunLoop` is (the `Stopper: Send` test already turns on that guarantee). `WakingSender<T>: Send` where `T: Send`, for the same reasons `mpsc::Sender<T>` is.

## Change 2: the title channel wakes on send

`crates/mercury/src/daemon.rs`, before:

```rust
    // Titles for the status item. The effect loop, on the worker, sends; the main thread applies
    // them on its next wake, because an NSStatusItem is main-thread-only. A std channel rather
    // than tokio's: the receiving end is the main thread, which is not in the runtime.
    let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();
```

after:

```rust
    // Titles for the status item. The effect loop, on the worker, sends; the main thread applies
    // them on its next wake, because an NSStatusItem is main-thread-only. A waking channel, so a
    // title change reaches the status item at once rather than at the next SLICE. A std channel
    // under the waker rather than tokio's: the receiving end is the main thread, not in the runtime.
    let waker = freddie_main_loop::MainWaker::new();
    let (title_tx, title_rx) = waker.channel::<&'static str>();
```

`title_tx` is now a `WakingSender<&'static str>`; its `send` has the same signature as `mpsc::Sender::send`, so the effect loop's send sites and the `title_rx.try_iter()` drain in `on_wake` are unchanged.

The `waker` binding is otherwise unused here, but keeping it is what makes the overlay change cheap: it hands the same `waker` to `freddie_overlay::overlay` so the overlay's channel wakes too. Until then, `waker.channel` is the only use, and the binding can be inlined as `freddie_main_loop::MainWaker::new().channel::<&'static str>()` if the overlay change is not next.

## What this unblocks

The overlay change (`refactors/pending/overlay-marshaling.md`) drains an overlay channel in `on_wake`. Built on a plain channel it would show the overlay up to `SLICE` late — a regression from the GCD dispatch it replaces, which wakes the run loop for free. Built on `WakingSender` it matches GCD's promptness with no `thread_local`. That doc is updated to take its channel from a `MainWaker` rather than a bare `mpsc::channel`, and to drop its "the latency this trades for" section, which was wrong: the trade only exists without this prefactor.
