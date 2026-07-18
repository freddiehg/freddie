# asking a running mercury what it is doing

Two read-only verbs. `mercury status` says whether a daemon is running and which process it is; `mercury logs` follows the log file. Neither starts anything, stops anything, or signals anything, and both are useful against a daemon started by hand in another pane.

Follows `single-instance-holder.md`, for `holder`, and `mercury-daemon-verb.md`, for the verb dispatch. Independent of `mercury-stop.md` and `mercury-start.md`, and can ship before either.

These are the first clients, so this is where `client.rs` starts. `cli.rs` holds what the command line says; `client.rs` holds what the verbs that are not the daemon do.

## `client.rs`

```rust
//! The client verbs: everything `mercury` does that is not being the daemon.
//!
//! None of these takes the single-instance lock. They probe it to find the daemon, and the daemon
//! is the only process that holds it.

use freddie_single_instance::Held;

/// The app name the lock is keyed to. The daemon acquires this; the clients probe it.
pub(crate) const APP: &str = "mercury";
```

`daemon::run` takes its lock through `freddie_single_instance::acquire(crate::client::APP)`, rather than repeating the `"mercury"` literal that is there today.

## `mercury status`

```rust
/// Report whether the daemon is running, and which process it is.
///
/// Exits 1 when nothing is running, so `mercury status && ...` reads the way a shell expects.
pub(crate) fn status() -> i32 {
    match freddie_single_instance::holder(APP) {
        Ok(Held::Free) => {
            println!("mercury is not running");
            1
        }
        Ok(Held::By(pid)) => {
            println!("mercury is running (pid {pid})");
            0
        }
        // The window between a daemon taking the lock and writing its pid into it. Something is
        // running, and that is the question this verb was asked.
        Ok(Held::Unnamed) => {
            println!("mercury is running (it has just started and has not recorded its pid)");
            0
        }
        Err(e) => {
            eprintln!("mercury: {e}");
            1
        }
    }
}
```

## `mercury logs`

```rust
/// Where `tail` lives. Absolute, so `PATH` cannot point this at something else.
const TAIL: &str = "/usr/bin/tail";

/// How much of the existing log to print before following.
const TAIL_LINES: &str = "50";

/// Follow the log file: print the tail of what is there, then whatever arrives.
///
/// `tail -F` rather than a follower of our own. It waits for a file that does not exist yet, which
/// is the first run on a machine before anything has been logged, and it reopens by name if the
/// file is replaced. Verified on macOS 25.5.0 against a path created a second after `tail`
/// started: the lines appear.
///
/// The child inherits this terminal's stdio and its process group, so Ctrl-C reaches it and ends
/// the follow. That is the whole reason `mercury-start.md` puts the daemon in a group of its own.
///
/// The log records `debug` whatever `--log-level` says, and this prints it verbatim; narrowing it
/// is a grep over the text, not a filter this can apply.
pub(crate) fn logs() -> i32 {
    let path = crate::logging::log_path();
    println!("mercury: following {}", path.display());
    match std::process::Command::new(TAIL)
        .args(["-n", TAIL_LINES])
        .arg("-F")
        .arg(&path)
        .status()
    {
        Ok(status) => i32::from(!status.success()),
        Err(e) => {
            eprintln!("mercury: could not run {TAIL}: {e}");
            1
        }
    }
}
```

## The log path without a subscriber

`logs` needs the path in a process that installs no subscriber and creates no file, which `init` cannot give it.

`crates/mercury/src/logging.rs`, before:

```rust
pub fn init(directives: &str) -> PathBuf {
    let dir = log_dir();
    // ...
    dir.join(LOG_FILE)
}
```

After:

```rust
/// Where the log file is, whether or not tracing has been initialized or anything has been written.
///
/// Separate from [`init`] because `mercury logs` needs the path without installing a subscriber of
/// its own: the daemon owns the log, and a client only reads it.
#[must_use]
pub fn log_path() -> PathBuf {
    log_dir().join(LOG_FILE)
}

pub fn init(directives: &str) -> PathBuf {
    let dir = log_dir();
    // ... unchanged ...
    log_path()
}
```

## Wiring

`Verb` gains two variants, ahead of `Daemon` in declaration order so they read first in `--help`:

```diff
 pub enum Verb {
+    /// Follow the log, starting nothing.
+    Logs,
+    /// Report whether the daemon is running, and its pid.
+    Status,
     /// Run the daemon in this terminal, in the foreground.
     Daemon(DaemonArgs),
 }
```

`main.rs` gains `mod client;` and two arms:

```diff
 fn run(verb: Option<Verb>) -> i32 {
     match verb {
         None => {
             daemon::run(DEFAULT_LOG_LEVEL);
             0
         }
+        Some(Verb::Logs) => client::logs(),
+        Some(Verb::Status) => client::status(),
         Some(Verb::Daemon(args)) => {
             daemon::run(&args.log_level);
             0
         }
     }
 }
```

Parse tests in `cli.rs`, matching the existing ones:

```rust
#[test]
fn the_read_only_verbs_parse() {
    assert!(matches!(parse(&["status"]).verb, Some(Verb::Status)));
    assert!(matches!(parse(&["logs"]).verb, Some(Verb::Logs)));
}
```

## Verifying

- `mercury status` with nothing running prints "mercury is not running" and exits 1.
- `cargo run -p mercury -- daemon` in another pane, then `mercury status` prints that pane's pid and exits 0.
- `mercury logs` prints the tail of the log and then follows it: switching layers in the daemon makes `dispatch` lines appear. Ctrl-C returns to the shell and leaves the daemon running.
- `mercury logs` on a machine that has never run mercury waits, printing nothing, and starts printing when a daemon first writes.
