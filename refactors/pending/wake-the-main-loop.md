# an event-driven main loop: wake on send, no idle poll

`MainLoop::run` parks the main thread in `nextEventMatchingMask_untilDate_inMode_dequeue` with a `SLICE` (100ms) deadline and runs `on_wake` each time it returns. That costs two things:

- Work handed to `on_wake` over a channel waits up to `SLICE`. A plain send does not break the block, so the value sits until the loop next surfaces. The menu-bar title pays this today; a keyboard-only layer change posts no `NSEvent`, so the title can update 100ms late.
- An idle loop wakes every `SLICE` to run `on_wake` over an empty channel and go back to sleep. Nothing is happening, and the thread is busy 10 times a second.

The stop is the same shape. `Stopper` breaks the loop with `CFRunLoop::stop`, which is a no-op against a loop that has not started, so a stop that races the block is lost and `SLICE` is the backstop that keeps main from hanging. `CFRunLoop::stop` is also unproven to break `nextEventMatchingMask` at all: the loop tolerates the `SLICE`, so a stop that never worked would look identical to one that did.

The loop should be event-driven: asleep at no cost when nothing is happening, and turning at once when something is. That needs one wake that both breaks `nextEventMatchingMask` reliably and cannot be lost if it races the block. A posted application-defined `NSEvent` is that wake: `nextEventMatchingMask` dequeues it and returns, and a post that lands before the loop blocks sits in the event queue and is dequeued the instant it does. With it, the loop blocks on `distantFuture`, `SLICE` is deleted, an idle loop truly sleeps, and a wake and a stop both reach main immediately through the same mechanism.

## The wake mechanism is confirmed

The design stands on one platform fact, verified in a scratch spike: a posted application-defined `NSEvent`, from a thread other than main, makes a main-thread `nextEventMatchingMask_untilDate_inMode_dequeue` return promptly. The spike parked the main thread on a 5-second deadline and, from a worker, posted an app-defined event with subtype `1` three times at 500ms spacing; `nextEventMatchingMask` returned at 503ms, 503ms, and 501ms — each posted event, not the deadline — carrying `NSEventType::ApplicationDefined` and `subtype` `Some(NSEventSubtype(1))`. `postEvent:atStart:` is callable off the main thread through objc2 (it takes `&self`, no `MainThreadMarker`), constructing the event off-main works, and the objc2 method names below compile as written. So `nextEventMatchingMask(distantFuture)` blocks until a real event or a posted wake, each posted wake breaks it, and a wake is told apart from a real event by its type and subtype — which is all the design needs.

## The wake handle

Posting needs `NSApp`, which is a process-global singleton reachable only on the main thread through `MainThreadMarker`. The handle captures it once, on the main thread, and is `Send` so the worker threads can poke. See `docs/platform-apis.md` for holding an OS resource this way.

`crates/freddie_main_loop/src/lib.rs`:

```rust
/// The subtype stamped on the events posted only to wake the loop, to tell them from real ones.
const WAKE_SUBTYPE: i16 = 1;

/// A handle to the process `NSApplication`, for posting a wake event from any thread.
///
/// `NSApp` outlives the process, so the pointer is stable, and `postEvent:atStart:` is thread-safe,
/// so a `&` call off the main thread is sound. Built on the main thread, where `NSApp` is reachable.
#[derive(Clone)]
struct AppHandle(NonNull<NSApplication>);

// SAFETY: `NSApp` lives for the whole process and `postEvent:atStart:` is documented thread-safe, so
// the pointer stays valid and the call is sound from any thread.
unsafe impl Send for AppHandle {}
unsafe impl Sync for AppHandle {}

impl AppHandle {
    fn new(app: &NSApplication) -> Self {
        Self(NonNull::from(app))
    }

    /// Post an application-defined event, which `nextEventMatchingMask` dequeues and returns.
    fn poke(&self) {
        // SAFETY: `self.0` is the process `NSApp`, valid for the process; `postEvent:atStart:` is
        // thread-safe; the event is a fresh application-defined event carrying `WAKE_SUBTYPE`.
        #[expect(unsafe_code)]
        unsafe {
            let app = self.0.as_ref();
            let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::ApplicationDefined,
                NSPoint::ZERO,
                NSEventModifierFlags::empty(),
                0.0,
                0,
                None,
                WAKE_SUBTYPE,
                0,
                0,
            );
            if let Some(event) = event {
                app.postEvent_atStart(&event, true);
            }
        }
    }
}
```

## `MainWaker` and `WakingSender`

`crates/freddie_main_loop/src/lib.rs`:

```rust
/// Wakes the main loop from any thread, so work queued for `on_wake` runs now. Cheap to clone; give
/// one to each thread that sends on a channel `on_wake` drains.
#[derive(Clone)]
pub struct MainWaker {
    app: AppHandle,
}

impl MainWaker {
    /// Wake the main loop: `nextEventMatchingMask` returns and `on_wake` runs.
    ///
    /// Send on the channel first, then call this: the send has to be visible when `on_wake` drains.
    pub fn wake(&self) {
        self.app.poke();
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

/// A channel sender that wakes the main loop after each send, so the value reaches `on_wake` at once.
/// `Send` and `Clone`. A consumer holds one of these instead of a bare `Sender`, so it cannot send
/// without waking; that is enforced here rather than left to each send site to remember.
pub struct WakingSender<T> {
    tx: Sender<T>,
    waker: MainWaker,
}

// Hand-written, not derived: `Sender<T>` is `Clone` for every `T` (a clone duplicates the handle,
// not a message), so this must be too. `#[derive(Clone)]` would add a spurious `T: Clone` bound,
// which `std`'s own `impl<T> Clone for Sender<T>` also avoids.
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

## Construction, `Stopper`, and the loop

The wake handle needs `NSApp`, so `main_loop` is built on the main thread and hands out the handle. It takes a `MainThreadMarker` and returns the waker beside the loop and the stopper.

`main_loop`, before:

```rust
pub fn main_loop() -> (MainLoop, Stopper) {
    let (signal, stop) = std::sync::mpsc::channel();
    let main_loop = MainLoop { stop };
    let stopper = Stopper {
        signal,
        run_loop: CFRunLoop::get_main(),
    };
    (main_loop, stopper)
}
```

after:

```rust
pub fn main_loop(mtm: MainThreadMarker) -> (MainLoop, Stopper, MainWaker) {
    let waker = MainWaker {
        app: AppHandle::new(&NSApplication::sharedApplication(mtm)),
    };
    let (signal, stop) = std::sync::mpsc::channel();
    (
        MainLoop { stop },
        Stopper {
            signal,
            waker: waker.clone(),
        },
        waker,
    )
}
```

`Stopper` holds a `MainWaker` and wakes through it instead of stopping the run loop; it no longer holds a `CFRunLoop`. A stop is a wake that also carries the exit signal. before:

```rust
pub struct Stopper {
    signal: Sender<()>,
    run_loop: CFRunLoop,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        let _ = self.signal.send(());
        self.run_loop.stop();
    }
}
```

after:

```rust
pub struct Stopper {
    signal: Sender<()>,
    // A `Stopper` is a `MainWaker` plus the exit signal: dropping it wakes the loop and tells it to
    // stop, where a bare wake tells it to keep going.
    waker: MainWaker,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        // Signal first, then wake, the same ordering as any waking send: the loop must see the stop
        // when the posted event breaks its wait. A post that lands before the loop blocks is not
        // lost, so there is no SLICE backstop and none is needed.
        let _ = self.signal.send(());
        self.waker.wake();
    }
}
```

`run` blocks on `distantFuture` and drops `SLICE`. The `SLICE` constant and the `core_foundation` run-loop imports go. before:

```rust
        while matches!(self.stop.try_recv(), Err(TryRecvError::Empty)) {
            autoreleasepool(|_| {
                #[expect(unsafe_code)]
                let event = unsafe {
                    let deadline = NSDate::dateWithTimeIntervalSinceNow(SLICE.as_secs_f64());
                    app.nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&deadline),
                        NSDefaultRunLoopMode,
                        true,
                    )
                };
                if let Some(event) = event {
                    app.sendEvent(&event);
                }
                on_wake();
            });
        }
```

after:

```rust
        while matches!(self.stop.try_recv(), Err(TryRecvError::Empty)) {
            autoreleasepool(|_| {
                // Block until something happens: a real window-server event, or a posted wake. No
                // deadline, so an idle loop sleeps at no cost.
                #[expect(unsafe_code)]
                // SAFETY: on the main thread; dequeuing one event, waiting indefinitely.
                let event = unsafe {
                    app.nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&NSDate::distantFuture()),
                        NSDefaultRunLoopMode,
                        true,
                    )
                };
                if let Some(event) = event {
                    // A wake event exists only to break the wait; do not dispatch it. Everything else
                    // is a real event the status item and menu tracking need.
                    if !is_wake_event(&event) {
                        app.sendEvent(&event);
                    }
                }
                on_wake();
            });
        }
```

and the predicate, beside `is_main_thread`:

```rust
/// Whether this is one of the events posted only to wake the loop.
fn is_wake_event(event: &NSEvent) -> bool {
    // The spike confirmed `subtype()` returns `Option<NSEventSubtype>`, a newtype over the `i16`.
    event.r#type() == NSEventType::ApplicationDefined
        && event.subtype() == Some(NSEventSubtype(WAKE_SUBTYPE))
}
```

The `SLICE` constant and its doc comment are deleted, along with `use core_foundation::...::CFRunLoop` and the `core-foundation` dependency if nothing else uses it.

## The tests change

The off-main tests exist because `main_loop` touched no `NSApp`. It does now, so they change:

- `dropping_the_stopper_signals_stop` tested the stop by dropping a `Stopper` off-main. The channel half of the stop is what it asserts, so it moves to constructing the `Sender`/`Receiver` directly and checking a send lands, without a `MainWaker`. The poke half is a main-thread `NSApp` call, covered by the spike, not by an off-main unit test.
- `the_stopper_is_send` stands: `MainWaker` is `Send` (its `AppHandle` is), so `Stopper` stays `Send`.
- `run_off_the_main_thread_panics` and `the_test_thread_is_not_main` are unchanged.

## Consumers

`crates/mercury/src/daemon.rs`. The title channel becomes a waking channel; the same `waker` is handed to the overlay (`refactors/pending/overlay-marshaling.md`). before:

```rust
    let (main_loop, stopper) = freddie_main_loop::main_loop();
    // ...
    let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();
```

after:

```rust
    let mtm = MainThreadMarker::new().expect("daemon::run is on the main thread");
    let (main_loop, stopper, waker) = freddie_main_loop::main_loop(mtm);
    // ...
    let (title_tx, title_rx) = waker.channel::<&'static str>();
```

`title_tx` is now a `WakingSender<&'static str>`; its `send` has the same signature as `mpsc::Sender::send`, so the effect loop's send sites and the `title_rx.try_iter()` drain in `on_wake` are unchanged. `init_menu_bar_app` still runs first, so `NSApp` exists when `main_loop(mtm)` builds the handle. The overlay change takes the same `waker`.

## What this buys

An idle daemon does nothing: the main thread sleeps in `nextEventMatchingMask` until a real event or a poke, with no periodic wakeups. A title change, an overlay show, or a stop reaches main the instant it is sent, through one mechanism. `SLICE` and the poll it drove are gone.
