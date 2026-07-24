# a panic tears the daemon down

Panics are bugs, not conditions to recover from. Freddie never catches a panic to continue, and there is no degraded mode. A panic anywhere brings the whole daemon down at once.

Today that is only half true. The daemon's work is split across two threads: the main thread parks in the run loop to service AppKit, and the worker runs the model and holds the keyboard `Interceptor`. Unwinding tears down only the thread that panicked.

- A worker panic unwinds the worker, which drops the `Interceptor` (releasing the tap) and the `Stopper` (stopping the main loop), so it happens to bring everything down.
- A main-thread panic — in an `on_wake` body, or an AppKit callback dispatched from the run loop — unwinds only main. The worker keeps running, holding the keyboard tap, with no one to stop it. The keyboard is wedged and the process does not exit.
- A panic unwinding through the C frames of an `AXObserver` or an AppKit callback is undefined behavior, whichever thread it is on.

The fix is to make every panic end the process, after logging it, before any unwinding.

## The mechanism: an aborting panic hook

`log_panics` (`crates/freddie_cli/src/logging.rs`) already installs a panic hook that routes the panic through `tracing`. It runs at the start of a panic, before the unwind. Aborting from it ends the process there, on any thread, before unwinding begins — so a main-thread panic takes the daemon down like a worker one, and a panic in a C callback aborts rather than unwinding through the C frame into undefined behavior.

`crates/freddie_cli/src/logging.rs`, before:

```rust
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panicked".to_owned());
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);
        let backtrace = std::backtrace::Backtrace::capture();
        tracing::error!(%location, %backtrace, "panic: {message}");
    }));
}
```

after:

```rust
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panicked".to_owned());
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);
        let backtrace = std::backtrace::Backtrace::capture();
        tracing::error!(%location, %backtrace, "panic: {message}");
        // A panic is a bug, and freddie's work is split across threads that unwinding cannot all
        // reach. End the process here, after the record is written, so no thread is left holding the
        // keyboard and no unwind runs through a C callback frame.
        std::process::abort();
    }));
}
```

The record has to reach the file before the abort. The daemon's file layer writes each record synchronously through `tracing_appender::rolling::never`, so the `error!` above is on disk by the time the hook returns to `abort`; confirm that when implementing, and flush explicitly if it is ever made non-blocking.

## Why abort and not `panic = "abort"`

`panic = "abort"` in a Cargo profile would do the same at the language level, but it fights the rest of the setup. On stable, `panic = "abort"` in `[profile.dev]` makes `cargo test` fail to build the harness, which needs unwinding, and freddie runs from dev builds (`bacon restart`), so the guarantee has to hold there, not only in release. The hook holds in every build because it is installed at runtime, and it leaves `cargo test` alone because tests never call `log_panics`: `init` installs it, and no test calls `init`, so `run_off_the_main_thread_panics`'s `catch_unwind` still catches under the default hook.

## What stays

- The panic is logged first. The hook writes the same `error!` record it does today, so the reason the daemon died still reaches the log and a client verb still shows it, before the abort.
- The keyboard frees without the `Interceptor`'s `Drop`. A `CGEvent` tap is removed by the OS when its process dies, so aborting releases the keyboard even though no destructor runs. Swallowed modifiers are not reopened, but a panic never reopened them — that is a model effect on a graceful `Kill`, not a destructor.
- `Stopper` still handles the graceful exits. A normal return and an early error return drop it and stop the loop through the channel; the abort covers only panics. The two paths are now separate: graceful stop drains and exits the loop, a panic ends the process on the spot.

## The one comment that changes

`crates/mercury/src/daemon.rs`, the `run` doc, before:

```rust
/// Dropping the worker's `Stopper` stops main's loop, so a normal return, a
/// failed keyboard grab, and a panic all exit. Declaration order below matters:
/// the runtime drops before the `Stopper`.
```

after:

```rust
/// Dropping the worker's `Stopper` stops main's loop, so a normal return and a failed keyboard grab
/// exit through it. A panic does not: it aborts the process from the panic hook (see
/// `log_panics`), which a `Stopper` on the worker could not do for a panic on the main thread.
/// Declaration order below matters: the runtime drops before the `Stopper`.
```
