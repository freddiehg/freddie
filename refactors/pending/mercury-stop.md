# stopping a mercury you cannot see

`mercury stop` ends a running daemon from another terminal, and it ends it the way the menu bar's Quit does rather than by killing it where it stands. Two pieces: the daemon learns to treat SIGTERM as a quit, and a client verb finds the daemon's pid and sends one.

Follows `single-instance-holder.md`, for the pid, and `mercury-daemon-verb.md`, for the verb dispatch. Independent of `mercury-status-and-logs.md`. `mercury-start.md` builds `restart` on the stopping half of this.

## Why a signal and not a socket message

`external-events.md` defines the loopback socket mercury listens on, and states that `IncomingEvent` names exactly what an outside sender may say so that "remote key injection and remote quit are unrepresentable rather than filtered". A quit frame is the thing that vocabulary exists to exclude, so `stop` does not go over the socket and the socket does not grow a variant for it.

## Change 1: the daemon quits on SIGTERM

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

This change is worth having on its own: `launchctl bootout` becomes a clean shutdown, and so does a `kill` typed by hand.

Verifying: `cargo run -p mercury -- daemon`, then `kill <pid>` from another pane. The log gets `SIGTERM: quitting` followed by `kill: exiting`, and the keyboard is normal afterwards.

## Change 2: `mercury stop`

In `client.rs`, alongside what `mercury-status-and-logs.md` put there.

```rust
use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};

/// How often the wait loops re-probe the lock.
const POLL: Duration = Duration::from_millis(50);

/// How long `stop` waits for the daemon to release the lock before reporting that it has not.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait out [`Held::Unnamed`], the window between a daemon locking the file and
/// writing its pid into it.
const PID_TIMEOUT: Duration = Duration::from_millis(500);

/// What a client found when it went looking for the daemon.
enum Target {
    /// Nothing holds the lock.
    NotRunning,
    /// The daemon, ready to be signalled.
    Running(Pid),
    /// Something holds the lock and never recorded a pid, so there is nothing to signal.
    Anonymous,
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

/// Ask the running daemon to quit, and wait for it to let go of the lock.
fn stop_daemon() -> Stopped {
    let pid = match find_daemon() {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Stopped::NotRunning,
        Ok(Target::Anonymous) => {
            eprintln!("mercury: something holds the lock but recorded no pid; it is starting or shutting down");
            return Stopped::Failed;
        }
        Err(e) => {
            eprintln!("mercury: {e}");
            return Stopped::Failed;
        }
    };
    if let Err(e) = terminate(pid) {
        eprintln!("mercury: could not signal pid {pid}: {e}");
        return Stopped::Failed;
    }
    if wait_until_free() {
        Stopped::Was(pid)
    } else {
        eprintln!("mercury: pid {pid} still holds the lock; run `kill -9 {pid}` if it is wedged");
        Stopped::Failed
    }
}

/// `mercury stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop() -> i32 {
    match stop_daemon() {
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

/// Send SIGTERM to `pid`, which is `/bin/kill`'s default signal.
///
/// A subprocess rather than a `kill(2)` binding, because the workspace forbids `unsafe` and every
/// binding for it is an unsafe extern call. The same trade `freddie_app_nav` makes by
/// foregrounding an app through `open`. An absolute path, so `PATH` cannot point this at something
/// else.
fn terminate(pid: Pid) -> io::Result<()> {
    let status = Command::new("/bin/kill").arg(pid.to_string()).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("/bin/kill exited with {status}")))
    }
}

/// Poll until nothing holds the lock, up to [`STOP_TIMEOUT`]. `true` when it was released.
///
/// The lock is what the wait is for, rather than the process disappearing: the daemon releases it
/// as it exits, and a released lock is exactly the condition under which the next start succeeds.
fn wait_until_free() -> bool {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if matches!(freddie_single_instance::holder(APP), Ok(Held::Free)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}
```

`stop` does not escalate to SIGKILL. A daemon that ignores SIGTERM is one whose event loop is wedged, and killing it uncleanly leaves exactly the state change 1 exists to avoid, so that decision is the user's and the message says how.

`Verb` gains `Stop`, and `main.rs` the arm calling `client::stop()`:

```diff
 pub enum Verb {
+    /// Ask the running daemon to quit.
+    Stop,
     /// Follow the log, starting nothing.
     Logs,
```

with a parse test beside the others.

## Verifying

- `cargo run -p mercury -- daemon` in one pane; `mercury stop` in another prints "mercury stopped (pid N)", the daemon's pane returns to the shell, and its log ends with `SIGTERM: quitting` then `kill: exiting`.
- `mercury stop` again prints "mercury is not running" and exits 0.
- Holding a command layer's modifier as the stop lands leaves no modifier stuck down in the app underneath, which is what change 1 buys.
