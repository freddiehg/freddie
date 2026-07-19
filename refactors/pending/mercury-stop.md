# stopping a mercury you cannot see

`mercury stop` ends a running daemon from another terminal, and it ends it the way the menu bar's Quit does rather than by destroying it where it stands. `mercury stop --force` is the second out, for a daemon that no longer answers. Three pieces: the lock crate learns to wait for a release, the daemon learns to treat SIGTERM as a quit, and a client verb finds the daemon's pid and signals it.

Follows `refactors/past/single-instance-holder.md`, for the pid, and `refactors/past/mercury-daemon-verb.md`, for the verb dispatch. Independent of `mercury-status-and-logs.md`. `mercury-start.md` builds `restart` on the stopping half of this.

## Why a signal and not a socket message

`external-events.md` defines the loopback socket mercury listens on, and states that `IncomingEvent` names exactly what an outside sender may say so that "remote key injection and remote quit are unrepresentable rather than filtered". A quit frame is the thing that vocabulary exists to exclude, so `stop` does not go over the socket and the socket does not grow a variant for it.

## Change 1: waiting for a lock to come free

`stop` has to know when the daemon is gone, and the lock is what it waits on rather than the process disappearing: the daemon releases it as it exits, and a released lock is exactly the condition under which the next start succeeds.

flock already reports that. A blocking `lock_shared` is granted the instant the exclusive holder lets go, so the wait is edge-triggered and needs no polling interval. Verified on the pinned 1.96.0: 48µs against a holder released after 300ms, where a 50ms poll would have averaged 25ms.

In `freddie_single_instance`, beside `holder` and `holder_at`:

```rust
/// Wait until nothing holds `app`'s lock.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory, and
/// otherwise whatever [`await_free_at`] returns.
pub fn await_free(app: &str) -> Result<(), LockError> {
    await_free_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Wait until nothing holds `path`'s lock, returning as it is released.
///
/// Blocks in flock, which grants the shared lock the moment the exclusive holder lets go. The
/// shared lock is dropped before this returns, so it leaves the path as it found it. Whether the
/// path is still free once the caller acts on the answer is not something this can promise, for
/// the reason [`holder_at`] gives.
///
/// There is no timeout, because flock has none to offer. A caller that needs one runs this on a
/// thread and stops listening to it.
///
/// # Errors
///
/// Returns [`LockError::Unavailable`] when the file cannot be created, opened, or locked.
pub fn await_free_at(path: &Path) -> Result<(), LockError> {
    let file = open(path)?;
    file.lock_shared().map_err(LockError::Unavailable)
}
```

Shared rather than exclusive for the same reason `holder_at` takes a shared lock: two clients waiting on the same daemon must not block each other, and an exclusive waiter would hold the path shut against the next daemon for as long as it took the caller to drop it.

Tests, beside the holder ones:

```rust
#[test]
fn waiting_blocks_until_the_holder_releases() {
    let path = temp_lock("await-release");
    let held = acquire_at(&path).expect("the path is free");
    let waiter = {
        let path = path.clone();
        std::thread::spawn(move || await_free_at(&path))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waiter.is_finished(), "the lock is still held");
    drop(held);
    waiter
        .join()
        .expect("the waiting thread")
        .expect("the lock came free");
}

#[test]
fn waiting_on_a_free_path_returns_at_once() {
    let path = temp_lock("await-free");
    await_free_at(&path).expect("nothing holds it");
}
```

This ships on its own, ahead of anything in mercury.

## Change 2: the daemon quits on SIGTERM

mercury installs no signal handler today, so a terminated process dies without running its `Drop` impls. `refactors/past/single-instance.md` records the observed consequence: "the log has a run with no `kill: exiting` line for exactly that reason." The keyboard is released by the kernel tearing the process down, but the modifiers a command layer swallowed are never re-opened, so the app on the other side is left believing a physically-held modifier is still down.

Routing SIGTERM into the event channel makes the way out the one that already exists: `quit` is bound at the root, it opens the held modifiers and pushes `Kill`, the effect loop breaks, and the `Interceptor` releases the keyboard on the way.

`crates/mercury/Cargo.toml`:

```diff
-tokio = { version = "1", features = ["rt", "macros", "sync", "time"] }
+tokio = { version = "1", features = ["rt", "macros", "signal", "sync", "time"] }
```

`crates/mercury/src/daemon.rs`, in `serve`, after the `freddie_app_nav` watcher is installed and before the `select!`:

```rust
    // `launchctl bootout` and `mercury stop` both send SIGTERM. Route it into the event channel as
    // the same Quit the menu bar sends, so a terminated mercury leaves the way it would have on
    // its own: the model turns it into `Kill`, the effect loop breaks, and the `Interceptor`
    // releases the keyboard.
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: quitting");
                    let _ = event_tx.send(quit_event());
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated mercury will not release the keyboard");
        }
    }
```

A spawned task rather than a third `select!` arm, because a `select!` arm that completed would drop the other two futures and skip the graceful path this exists to run.

The runtime is already `new_current_thread().enable_all()`, which drives the signal handler. Verified to build and run on the pinned 1.96.0.

`launchctl bootout` becomes a clean shutdown out of this, and so does a `kill` typed by hand.

### What this costs

Installing the handler replaces SIGTERM's default disposition, which the kernel honours unconditionally, with one that depends on the runtime still being scheduled. `perform_effect` makes synchronous calls on the worker thread — `freddie_overlay::show`, `emitter.tap` — and tokio's signal driver is part of the runtime, so a worker blocked inside one of those never completes `term.recv().await` and never sends the quit.

Measured on the pinned 1.96.0, against a current-thread runtime with a signal task shaped like the one above:

```
healthy worker:  READY -> HANDLER RAN -> GRACEFUL EXIT -> exited on SIGTERM
blocked worker:  READY -> SURVIVED SIGTERM (handler never ran) -> died on SIGKILL
```

So a daemon blocked in an effect dies to `kill` before this change and survives it afterwards. That is what `--force` in change 3 is for, and why the escape hatch ships in the same doc as the thing that closes the old one.

Verifying: `cargo run -p mercury -- daemon`, then `kill <pid>` from another pane. The log gets `SIGTERM: quitting` followed by `kill: exiting`, and the keyboard is normal afterwards.

## Change 3: `mercury stop`

In `client.rs`, alongside what `mercury-status-and-logs.md` put there.

```rust
use std::io;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};

/// How often [`find_daemon`] re-probes a lock whose holder has not named itself yet.
const POLL: Duration = Duration::from_millis(10);

/// How long `stop` waits for the daemon to release the lock before reporting that it has not.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait out [`Held::Unnamed`], the window between a daemon locking the file and
/// writing its pid into it.
///
/// Ten polls. The window is a `set_len` and a `write_all` on an already-open file, so it closes in
/// microseconds, and a holder still anonymous after this is one no longer than wait will help:
/// either its pid write failed, which fails its acquire and releases the lock, or it has been
/// stopped mid-acquire and will not finish at all. Failing fast reports that; waiting only delays
/// it.
const PID_TIMEOUT: Duration = Duration::from_millis(100);

/// What a client found when it went looking for the daemon.
enum Target {
    /// Nothing holds the lock.
    NotRunning,
    /// The daemon, ready to be signalled.
    Running(Pid),
    /// Something holds the lock and never recorded a pid, so there is nothing to signal.
    Anonymous,
}

/// The signal `stop` sends, and what each one costs.
#[derive(Clone, Copy)]
enum Signal {
    /// SIGTERM. The daemon routes it into the event channel and leaves the way the menu bar's Quit
    /// does, opening the modifiers it swallowed on the way out.
    Terminate,
    /// SIGKILL. The kernel destroys the process, so no destructor runs, the keyboard grab is torn
    /// down rather than released, and a swallowed modifier stays down in the app underneath. The
    /// only out for a daemon whose worker is blocked in an effect, which SIGTERM cannot reach.
    Kill,
}

impl Signal {
    /// How `/bin/kill` spells it.
    const fn flag(self) -> &'static str {
        match self {
            Self::Terminate => "-TERM",
            Self::Kill => "-KILL",
        }
    }
}

/// What [`stop_daemon`] found and did.
///
/// `stop` and `restart` both need the outcome and word it differently, so the stopping reports the
/// fact and each caller phrases its own line. The failures are worded inside `stop_daemon`,
/// because a daemon that would not go away reads the same whichever verb asked.
enum Stopped {
    /// A daemon was signalled and let go of the lock.
    Was(Pid),
    /// There was nothing to stop.
    NotRunning,
    /// A daemon is there and did not go away, or could not be found to signal.
    Failed,
}

/// Ask the running daemon to go, and wait for it to let go of the lock.
fn stop_daemon(signal: Signal) -> Stopped {
    let pid = match find_daemon() {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Stopped::NotRunning,
        Ok(Target::Anonymous) => {
            eprintln!(
                "mercury: something holds the lock but recorded no pid; it is starting or shutting down"
            );
            return Stopped::Failed;
        }
        Err(e) => {
            eprintln!("mercury: {e}");
            return Stopped::Failed;
        }
    };
    // Before the signal, so the wait cannot miss a daemon that exits between the two.
    let freed = watch_for_free();
    if let Err(e) = signal_pid(pid, signal) {
        eprintln!("mercury: could not signal pid {pid}: {e}");
        return Stopped::Failed;
    }
    if matches!(freed.recv_timeout(STOP_TIMEOUT), Ok(Ok(()))) {
        Stopped::Was(pid)
    } else {
        eprintln!("mercury: pid {pid} still holds the lock; `mercury stop --force` destroys it");
        Stopped::Failed
    }
}

/// `mercury stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop(args: &StopArgs) -> i32 {
    let signal = if args.force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(signal) {
        Stopped::Was(pid) => {
            println!("mercury stopped (pid {pid})");
            0
        }
        Stopped::NotRunning => {
            println!("mercury is not running");
            0
        }
        Stopped::Failed => 1,
    }
}

/// Find the daemon, waiting out the window in which it holds the lock without having named itself.
///
/// That window is a `set_len` and a `write_all` on an already-open file, so it closes in
/// microseconds. A holder that stays anonymous past [`PID_TIMEOUT`] is one whose pid write failed,
/// which `acquire_at` treats as a failure to start, so its lock is about to be released anyway.
///
/// Polled rather than waited on, unlike [`watch_for_free`]: the lock is held throughout this
/// window, so flock has nothing to report and there is no edge to wait for.
fn find_daemon() -> Result<Target, LockError> {
    let deadline = Instant::now() + PID_TIMEOUT;
    loop {
        match freddie_single_instance::holder(APP)? {
            Held::Free => return Ok(Target::NotRunning),
            Held::By(pid) => return Ok(Target::Running(pid)),
            Held::Unnamed if Instant::now() >= deadline => return Ok(Target::Anonymous),
            Held::Unnamed => std::thread::sleep(POLL),
        }
    }
}

/// Start waiting for the lock to come free, and hand back the channel it reports on.
///
/// `await_free` blocks in flock with no timeout of its own, so it runs on a thread and the caller
/// stops listening once its own deadline passes. Abandoning that thread costs nothing: `stop`
/// exits moments later and the process teardown takes it along.
fn watch_for_free() -> mpsc::Receiver<Result<(), LockError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(freddie_single_instance::await_free(APP));
    });
    rx
}

/// Send `signal` to `pid`.
///
/// A subprocess rather than a `kill(2)` binding, because the workspace forbids `unsafe` and every
/// binding for it is an unsafe extern call. The same trade `freddie_app_nav` makes by
/// foregrounding an app through `open`. An absolute path, so `PATH` cannot point this at something
/// else.
fn signal_pid(pid: Pid, signal: Signal) -> io::Result<()> {
    let status = Command::new("/bin/kill")
        .arg(signal.flag())
        .arg(pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("/bin/kill exited with {status}")))
    }
}
```

`stop` does not escalate to SIGKILL on its own. The graceful path exists to reopen the modifiers a command layer swallowed, and a client that quietly destroyed the daemon after five seconds would throw that away without saying so. `--force` is the same act, asked for.

`cli.rs` gains the verb and its argument:

```rust
pub enum Verb {
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
}

/// What `mercury stop` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct StopArgs {
    /// Destroy the daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so a modifier the command
    /// layer swallowed is left down in whatever app was in front.
    #[arg(long)]
    pub force: bool,
}
```

and `main.rs` the arm:

```rust
        Some(Verb::Stop(args)) => client::stop(&args),
```

with parse tests beside the others:

```rust
    #[test]
    fn stop_is_gentle_by_default() {
        let Some(Verb::Stop(args)) = parse(&["stop"]).verb else {
            panic!("the stop verb parses to Verb::Stop");
        };
        assert!(!args.force);
    }

    #[test]
    fn stop_takes_force() {
        let Some(Verb::Stop(args)) = parse(&["stop", "--force"]).verb else {
            panic!("the stop verb parses to Verb::Stop");
        };
        assert!(args.force);
    }
```

## Verifying

- `cargo run -p mercury -- daemon` in one pane; `mercury stop` in another prints "mercury stopped (pid N)", the daemon's pane returns to the shell, and its log ends with `SIGTERM: quitting` then `kill: exiting`.
- `mercury stop` again prints "mercury is not running" and exits 0.
- Holding a command layer's modifier as the stop lands leaves no modifier stuck down in the app underneath, which is what change 2 buys.
- `mercury stop --force` against a running daemon ends it with no `kill: exiting` line, which is the cost stated on `Signal::Kill`.
