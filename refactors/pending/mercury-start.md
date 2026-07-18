# starting a mercury that outlives the terminal

`mercury` starts a daemon detached from the terminal that started it and then follows its log, so Ctrl-C ends the log stream and leaves mercury running. `mercury start` does the starting alone, and `mercury restart` replaces a running daemon with a fresh one.

This is the last of the five and the one the others exist for. It follows `mercury-daemon-verb.md` for the verb to spawn, `single-instance-holder.md` for finding what is already running, `mercury-status-and-logs.md` for the log follower the bare `mercury` hands off to, and `mercury-stop.md` for the stopping half of `restart`.

## Detaching without `unsafe`

The daemon is spawned as `current_exe daemon` in a process group of its own.

`current_exe` rather than a configured path, so the child is whatever binary the user invoked: `cargo run -p mercury` starts the debug build under `target/`, and an installed binary starts itself, with no path to keep in step.

`process_group(0)` is what makes Ctrl-C on the log stream not take the daemon with it. Without it the child stays in the terminal's foreground process group and receives the same SIGINT the follower does. It also keeps the daemon off the SIGHUP the kernel sends the session's foreground group when the terminal closes. It is a safe method on `std::os::unix::process::CommandExt`, so this costs no `unsafe` and no `libc`, which matters against a workspace that forbids the former. Verified on the pinned 1.96.0.

Not a full session leader: that needs `setsid`, which macOS does not ship as a binary and which is an unsafe call from Rust. A process group is enough for both signals that would otherwise reach it, and `launch-at-login.md` is the answer for a daemon that should outlive the login session entirely.

## Change 1: `mercury start`

In `client.rs`.

```rust
/// How long `start` waits for a spawned daemon to take the lock.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a daemon is up, once `start` is done trying.
enum Started {
    /// One is running: this call started it, or found it already there.
    Running,
    /// None is running and this call could not start one.
    Failed,
}

/// Start the daemon unless one is already running.
///
/// The check before spawning is for the message, not for mutual exclusion. Two `start`s at the
/// same instant can both see [`Held::Free`] and both spawn, and the lock refuses one of the two
/// daemons exactly as it refuses a second `mercury daemon`; nothing here has to be atomic.
fn start() -> Started {
    match freddie_single_instance::holder(APP) {
        Ok(Held::By(pid)) => {
            println!("mercury is already running (pid {pid})");
            return Started::Running;
        }
        Ok(Held::Unnamed) => {
            println!("mercury is already running");
            return Started::Running;
        }
        Ok(Held::Free) => {}
        Err(e) => {
            eprintln!("mercury: {e}");
            return Started::Failed;
        }
    }
    match spawn_daemon() {
        Ok(pid) => {
            if wait_until_held() {
                println!("mercury started (pid {pid})");
                Started::Running
            } else {
                eprintln!(
                    "mercury: the daemon did not start; see {}",
                    crate::logging::log_path().display()
                );
                Started::Failed
            }
        }
        Err(e) => {
            eprintln!("mercury: could not start the daemon: {e}");
            Started::Failed
        }
    }
}

/// Spawn this same binary as `mercury daemon`, detached from this terminal.
///
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer then has nowhere
/// to write, which is why `--log-level` sits on the `daemon` verb and is not passed through here:
/// it governs a terminal this child does not have. The log file records `debug` regardless, and
/// `mercury logs` reads that.
fn spawn_daemon() -> io::Result<u32> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(child.id())
}

/// Poll until something holds the lock, up to [`START_TIMEOUT`]. `true` when one does.
///
/// Taking the lock is the readiness signal, and the daemon takes it first thing, before it
/// measures the screens, shows an icon, or grabs the keyboard. So this returning `true` says the
/// process is alive and is the one mercury, not that the keyboard is grabbed: a daemon refused
/// Accessibility fails a moment later and says so in the log this points at.
fn wait_until_held() -> bool {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if !matches!(freddie_single_instance::holder(APP), Ok(Held::Free)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// `mercury start`: make sure a daemon is up, and do not stay to watch.
pub(crate) fn start_only() -> i32 {
    match start() {
        Started::Running => 0,
        Started::Failed => 1,
    }
}
```

`Verb` gains `Start` as its first variant, and `main.rs` the arm calling `client::start_only()`.

## Change 2: `mercury restart`

```rust
/// `mercury restart`: replace the running daemon with a fresh one.
///
/// The two halves are already sequenced by the lock. `stop_daemon` returns only once the lock is
/// free, which is the same condition `start` needs to find, so the new daemon never races the old
/// one's shutdown and reports "already running" against the process it just killed.
///
/// A daemon that would not stop means no start is attempted: the old process still owns the tap,
/// and spawning a second one that the lock immediately refuses would say nothing useful.
///
/// Starting from cold is a restart with an empty first half, rather than an error, so a script
/// that restarts after a rebuild does not have to know whether anything was up.
pub(crate) fn restart() -> i32 {
    match stop_daemon() {
        Stopped::Was(pid) => println!("mercury stopped (pid {pid})"),
        Stopped::NotRunning => println!("mercury was not running"),
        Stopped::Failed => return 1,
    }
    start_only()
}
```

This is the verb a rebuild wants: `cargo build -p mercury && ./target/debug/mercury restart` replaces a running daemon with the binary just built. It stops and starts rather than signalling the daemon to re-exec itself, because the running process is the old binary and nothing about it knows a new one exists.

`Verb` gains `Restart`, after `Start`.

## Change 3: the bare `mercury` starts and follows

```rust
/// The bare `mercury`: make sure a daemon is up, then watch what it does.
pub(crate) fn start_and_follow() -> i32 {
    match start() {
        Started::Running => logs(),
        Started::Failed => 1,
    }
}
```

`main.rs`, where the bare verb has run the daemon in the foreground until now:

```diff
 fn run(verb: Option<Verb>) -> i32 {
     match verb {
-        None => {
-            daemon::run(DEFAULT_LOG_LEVEL);
-            0
-        }
+        None => client::start_and_follow(),
         Some(Verb::Start) => client::start_only(),
```

`DEFAULT_LOG_LEVEL` goes back to being nothing but `DaemonArgs`'s default, since the bare `mercury` no longer runs a daemon in this process.

The finished `Verb`, in help order:

```rust
pub enum Verb {
    /// Start the daemon if it is not running, and exit.
    Start,
    /// Stop the running daemon and start a fresh one.
    Restart,
    /// Follow the log, starting nothing.
    Logs,
    /// Ask the running daemon to quit.
    Stop,
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
}
```

`daemon` last, as the one a user rarely types. The test that changes is the one naming what the bare command does:

```rust
#[test]
fn no_verb_starts_and_follows() {
    assert!(parse(&[]).verb.is_none());
}
```

`None` is still what the bare `mercury` parses to; what moved is the arm in `run` that interprets it.

## Verifying

From a shell with no mercury running:

- `mercury` prints "mercury started (pid N)" and then follows the log. Ctrl-C returns to the shell; `mercury status` still reports pid N, and the keyboard still remaps.
- `mercury` again prints "mercury is already running (pid N)" and follows.
- Closing the terminal entirely leaves the daemon up, by `mercury status` from a new one.
- `mercury restart` prints "mercury stopped (pid N)" then "mercury started (pid M)" with `M` a different pid, and `mercury status` reports `M`. The log shows the old daemon's `SIGTERM: quitting` and `kill: exiting` before the new one's `initial state`.
- `mercury restart` with nothing running prints "mercury was not running" and starts one.
- Editing a binding, then `cargo build -p mercury && ./target/debug/mercury restart`, and the new binding is live without any window having been touched.
- `tccutil reset Accessibility` and then `mercury`: the daemon takes the lock, the follower shows the `could not intercept the keyboard` line, and the process exits.

## What `launch-at-login.md` inherits

Its plist runs `mercury daemon`, and `launchctl bootout` is a clean shutdown through `mercury-stop.md`'s SIGTERM handling. That doc's open question about a hand-started mercury fighting the loaded agent is answered by the lock: `mercury start` finds the agent's daemon holding it and starts nothing.
