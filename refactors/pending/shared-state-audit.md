# bringing the shared-state sites in line with the standard

`CLAUDE.md`'s "Shared state and interior mutability" section says `Arc`, `Rc`, `Mutex`, `RwLock`, `Cell`, `RefCell`, `OnceLock`, `lazy_static`, `thread_local!`, and atomics are almost always the wrong reach: the model is a single-threaded pure function of state and event, and the preferred way to move data between threads is a channel whose sender is freely `Send` while the receiver stays pinned to one thread.

Seven files predate that rule. This doc lists every one, changes the ones that fail the standard, and records the verdict on the ones the constraint genuinely forces, so a later audit does not re-open them.

Full survey of the tree: the ten other crates (`bind`, `bind_macro`, `derive_support`, `freddie_keys`, `freddie_keyboard`, `freddie_menu_bar`, `freddie_single_instance`, `freddie_app_nav`, `laserbeam`, `mercury`) use none of these primitives.

## Change 1: `freddie_main_loop`'s stop signal becomes a channel

This is the exact shape the rule names as preferred, built from the primitive it names as wrong. `Stopper` holds the write end and is `Send` (it crosses to the worker thread); `MainLoop` holds the read end and never leaves the main thread. That is sender-freely-`Send`, receiver-pinned-to-one-thread. A `std::sync::mpsc` channel replaces the `Arc<AtomicBool>`, matching the title channel `daemon.rs` already uses for the same reason ("the receiving end is the main thread, which is not in the runtime").

The public API is unchanged: `main_loop() -> (MainLoop, Stopper)` keeps its signature, so `crates/mercury/src/daemon.rs` is untouched.

`crates/freddie_main_loop/src/lib.rs`, imports, before:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
```

after:

```rust
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
```

The constructor, before:

```rust
pub fn main_loop() -> (MainLoop, Stopper) {
    let stop = Arc::new(AtomicBool::new(false));
    let main_loop = MainLoop {
        stop: Arc::clone(&stop),
    };
    let stopper = Stopper {
        stop,
        run_loop: CFRunLoop::get_main(),
    };
    (main_loop, stopper)
}
```

after:

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

`MainLoop`, before:

```rust
#[must_use = "the main loop does nothing until it is run"]
pub struct MainLoop {
    stop: Arc<AtomicBool>,
}
```

after:

```rust
#[must_use = "the main loop does nothing until it is run"]
pub struct MainLoop {
    stop: Receiver<()>,
}
```

The loop condition in `run`, before:

```rust
        while !self.stop.load(Ordering::Acquire) {
            autoreleasepool(|_| {
```

after:

```rust
        // `Empty` is the only reason to keep turning. A buffered `()` means the stopper sent
        // before dropping; `Disconnected` means it dropped without sending, which cannot happen
        // while its `Drop` sends, but is still a stop.
        while matches!(self.stop.try_recv(), Err(TryRecvError::Empty)) {
            autoreleasepool(|_| {
```

`Stopper`, before:

```rust
pub struct Stopper {
    stop: Arc<AtomicBool>,
    run_loop: CFRunLoop,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        // The flag first: `CFRunLoop::stop` against a loop that has not started is
        // a no-op, so a worker that dies before `MainLoop::run` is reached must
        // leave something behind for it to find.
        self.stop.store(true, Ordering::Release);
        // And the stop, to break it out of the current slice if it has started.
        self.run_loop.stop();
    }
}
```

after:

```rust
pub struct Stopper {
    signal: Sender<()>,
    run_loop: CFRunLoop,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        // The send first, then the run-loop stop. `CFRunLoop::stop` wakes the main thread out of
        // its slice; the send has to be visible by the time it wakes and re-checks, or the loop
        // would see `Empty` and block again. The buffered `()` also covers a worker that dies
        // before `MainLoop::run` is reached: `stop` is then a no-op and the message is what `run`
        // finds on its first pass.
        let _ = self.signal.send(());
        self.run_loop.stop();
    }
}
```

Tests, before:

```rust
    // Dropping the stopper before the loop runs must still stop it. The flag is
    // what carries that, since CFRunLoopStop against a loop that has not started
    // is a no-op. Asserted on the flag rather than by running a loop, because
    // `cargo test` has no main thread to give.
    #[test]
    fn dropping_the_stopper_sets_the_flag() {
        let (main_loop, stopper) = main_loop();
        assert!(!main_loop.stop.load(std::sync::atomic::Ordering::Acquire));
        drop(stopper);
        assert!(main_loop.stop.load(std::sync::atomic::Ordering::Acquire));
    }
```

after:

```rust
    // Dropping the stopper before the loop runs must still stop it. The buffered send carries
    // that, since CFRunLoopStop against a loop that has not started is a no-op. Asserted on the
    // channel rather than by running a loop, because `cargo test` has no main thread to give.
    #[test]
    fn dropping_the_stopper_signals_stop() {
        let (main_loop, stopper) = main_loop();
        assert!(matches!(
            main_loop.stop.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(stopper);
        assert!(main_loop.stop.try_recv().is_ok());
    }
```

`the_stopper_is_send`, `run_off_the_main_thread_panics`, and `the_test_thread_is_not_main` are unchanged: `Sender<()>` is `Send`, so `Stopper` stays `Send`.

## Change 2: `freddie/src/timer.rs`'s id source

`crates/freddie/src/timer.rs:29`:

```rust
fn mint() -> Self {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Self(NEXT.fetch_add(1, Ordering::Relaxed))
}
```

The comment already admits the atomic is not synchronizing anything: "Atomic because a mutable static has to be `Sync`, not because anything sets timers off one thread." It is a process-global mutable counter touched from one thread.

This needs a decision before it is written, because the two ways to remove it are not equivalent:

- `thread_local! { static NEXT: Cell<u64> }`, mirroring `freddie_overlay`'s `NEXT_ID`. Small and local, but trades one listed primitive for two, and keeps the id source as ambient global state rather than model state.
- The counter lives in the model and is threaded to `timer_effect_and_guard`. This is the model-pure answer: minting is already the one impure thing a handler does, so the next id belongs in state beside everything else a dispatch reads. It is the larger change: every `arm_*` helper in `crates/mercury/src/state/mod.rs` (and the `arm_*` in `app.rs`, `resize.rs`, `site.rs`, `nav.rs`) that calls `timer_effect_and_guard` gains access to the counter, and `TimerId::mint` stops being a free function.

Callers to touch under the second option: `crates/mercury/src/state/mod.rs:49,70,345,625`, and the test helpers in `crates/mercury/tests/transitions.rs:26,33,1209,1448`.

Per the standard's own directive ("raise it with the user, every single time"), the direction is the user's to pick. Once picked, the full before/after belongs here.

## Change 3: `freddie_overlay`'s marshaling to the main thread

`crates/freddie_overlay/src/lib.rs:46`:

```rust
thread_local! {
    static PANELS: RefCell<HashMap<OverlayId, Panel>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}
```

The justification is real: a block dispatched to the main GCD queue must be `'static + Send`, so it cannot carry an `NSPanel` and looks one up in the table instead. But `freddie_main_loop::MainLoop::run` already exists to run main-thread-only work handed in from elsewhere, through its `on_wake`, and `daemon.rs` already drains a title channel there. `OverlaySink::show`/`hide` could send over a channel drained on `on_wake` rather than dispatching into a thread-local table, which deletes both the `RefCell` table and the GCD marshaling.

This is a larger change than the other two: it replaces the sink's whole delivery path, touches `daemon.rs`'s `on_wake` closure, and has to decide what the sink sends and how the panel set is owned on the main thread. It should be its own doc once the direction is set, per `CLAUDE.md`'s rule on splitting a refactor that is too large. Flagged here so the audit record is whole; not specified here.

## Change 4: `freddie_event_socket`'s handler sharing

`crates/freddie_event_socket/src/lib.rs:63`:

```rust
let on_message = Arc::new(on_message);
// ...
tokio::spawn(serve(stream, Arc::clone(&on_message), closed.clone()));
```

`Arc<F>` shares one immutable `Fn` across the connection tasks. This is read-only sharing, the mildest use, and idiomatic for a handler fanned out over tokio tasks. It could be restructured so one task owns `F` and each connection forwards its frames over an `mpsc` (`on_message` is already required non-blocking, so funneling through one task costs nothing), removing the `Arc`. Low priority, and a decision to make rather than a defect. Left as it is unless the user wants the restructure.

## Reviewed and kept

These sites hold a primitive because a platform or trait constraint forces it. Each gets one line added to the type or function's doc comment recording that the constraint was reviewed against the standard and stands, so the next audit does not re-litigate.

- `crates/freddie_windows/src/lib.rs:556` `elements: Arc<Elements>` where `Elements(Mutex<HashMap<WindowId, Arc<Element>>>)`. The one genuinely cross-thread case. `WindowSink::set_frame` runs off the main thread by design (tens of milliseconds per placement, which the run loop cannot spend) while the main thread mutates the table as windows open and close. A channel to the main thread would put those milliseconds back on the run loop, which is the thing the design avoids.
- `crates/freddie_windows/src/lib.rs:976,559` `state: Rc<WatcherState>` and `apps: RefCell<HashMap<Pid, AppObserver>>`. The C-callback-reaches-state pattern from `docs/platform-apis.md`: `AXObserver` callbacks reach state through a `refcon`, and the `'static` launch and terminate closures cannot borrow the `Watcher`. Single-threaded shared ownership with C callbacks.
- `crates/freddie_cli/src/logging.rs:86` `thread_local! { static STAMPED: RefCell<Vec<u8>> }`. `io::Write::write` is a synchronous trait method `tracing` calls on whatever thread logs, so there is no owning thread to send to. The buffer is per-thread scratch reused to avoid a per-record allocation on the daemon's hot path.
- `crates/freddie_cli/src/logging.rs:94` `static STAMP: OnceLock<String>`. The process pid string, built once and immutable. This one could instead be computed at `init` and owned inside `WithPid<W>`, removing the `OnceLock`; fold that in when `logging.rs` is next touched, but it is not worth a standalone change.
- `crates/freddie_event_socket/tests/socket.rs:19` `Arc<Mutex<Vec<String>>>`. A test recorder. `CLAUDE.md` scopes the model standard to the model, and a test that collects what a callback saw is outside it.
