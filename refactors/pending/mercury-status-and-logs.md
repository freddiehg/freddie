# asking a running mercury what it is doing

Two read-only verbs. `mercury status` says whether a daemon is running and which process it is; `mercury logs` follows the log file. Neither starts anything, stops anything, or signals anything, and both are useful against a daemon started by hand in another pane.

Follows `refactors/past/mercury-stop.md`, which brought `client.rs` and its `APP` constant into being, and `refactors/past/single-instance-holder.md`, for `holder`. `mercury-start.md` builds the bare `mercury`'s log follower on `logs`.

## Read-only verbs leave no record

`stop` initializes logging and traces what it did, because it ended a process and the log is where the next person looks to find out why. These two change nothing, so they write nothing: no `logging::init`, no tracing, and errors on stderr only.

That is not only tidiness. `status` is the verb a shell prompt or a watch loop runs on a timer, and a client that logged every probe would fill the daemon's own log with records of being asked whether it was alive. `logs` is worse, since it follows the file it would be writing to.

## `mercury status`

```rust
/// Report whether the daemon is running, and which process it is.
///
/// Exits 1 when nothing is running, so `mercury status && ...` reads the way a shell expects. That
/// is the opposite of `stop`, which exits 0 having found nothing to stop, and both are deliberate:
/// this verb answers a question, so its exit code is the answer, while `stop` states a goal that a
/// stopped mercury already satisfies.
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
        // running, and that is the question this verb was asked, so it answers yes without the
        // pid rather than waiting for one.
        //
        // `stop` treats the same state as a failure, because a signal needs a pid and there is
        // none. Neither is wrong: they are asking the lock different questions.
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

`use freddie_single_instance::Held;` joins the imports `client.rs` already has.

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

`Verb` gains two variants. Declaration order is help order, and the order is the verbs that talk to a daemon before the one that becomes it, so the two most-run verbs are the first two lines of `mercury --help`.

`crates/mercury/src/cli.rs`, before:

```rust
pub enum Verb {
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
}
```

After:

```rust
pub enum Verb {
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Follow the log, starting nothing.
    Logs,
    /// Ask the running daemon to quit.
    Stop(StopArgs),
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
}
```

`crates/mercury/src/main.rs`, before:

```rust
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        None => {
            daemon::run(&DaemonArgs::default());
            0
        }
        Some(Verb::Daemon(args)) => {
            daemon::run(&args);
            0
        }
        Some(Verb::Stop(args)) => client::stop(&args),
    }
}
```

After:

```rust
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        None => {
            daemon::run(&DaemonArgs::default());
            0
        }
        Some(Verb::Status) => client::status(),
        Some(Verb::Logs) => client::logs(),
        Some(Verb::Stop(args)) => client::stop(&args),
        Some(Verb::Daemon(args)) => {
            daemon::run(&args);
            0
        }
    }
}
```

Parse tests in `cli.rs`, beside the existing ones:

```rust
#[test]
fn the_read_only_verbs_parse() {
    assert!(matches!(parse(&["status"]).verb, Some(Verb::Status)));
    assert!(matches!(parse(&["logs"]).verb, Some(Verb::Logs)));
}
```

## Verifying

- `mercury status` with nothing running prints "mercury is not running" and exits 1.
- `mercury daemon` in another pane, then `mercury status` prints that pane's pid and exits 0, and the pid matches the one `mercury stop` reports when it ends it.
- `mercury status` writes nothing to `~/Library/Logs/mercury/mercury.log`: the file's line count is the same before and after.
- `mercury logs` prints the tail of the log and then follows it: switching layers in the daemon makes `dispatch` lines appear. Ctrl-C returns to the shell and leaves the daemon running.
- `mercury logs` on a machine that has never run mercury waits, printing nothing, and starts printing when a daemon first writes.
