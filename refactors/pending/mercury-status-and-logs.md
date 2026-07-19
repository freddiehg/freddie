# asking a running mercury what it is doing

Two read-only verbs. `mercury status` says whether a daemon is running and which process it is; `mercury logs` follows the log file. Neither starts anything, stops anything, or signals anything, and both are useful against a daemon started by hand in another pane.

Follows `refactors/past/mercury-stop.md`, which brought `client.rs` and its `APP` constant into being, and `refactors/past/single-instance-holder.md`, for `holder`. `mercury-start.md` builds the bare `mercury`'s log follower on `logs`.

## Both verbs speak through tracing

`one-log-many-writers.md` makes the terminal a layer and the file the record of everything, so these two say their lines with `info!` and `warn!` like the daemon does, and both lines land in the file as well as on the terminal. Neither prints.

The one thing that does not go through tracing is `tail`'s output under `logs`. Those lines came out of the file, so emitting them as records would append them back into the file being followed, which would then show them again. `logs` displays what it reads; it does not re-log it.

## `mercury status`

```rust
/// Report whether the daemon is running, and which process it is.
///
/// Exits 1 when nothing is running, so `mercury status && ...` reads the way a shell expects. That
/// is the opposite of `stop`, which exits 0 having found nothing to stop, and both are deliberate:
/// this verb answers a question, so its exit code is the answer, while `stop` states a goal that a
/// stopped mercury already satisfies.
pub(crate) fn status() -> i32 {
    logging::init(Terminal::Client);
    match freddie_single_instance::holder(APP) {
        Ok(Held::Free) => {
            info!("mercury is not running");
            1
        }
        Ok(Held::By(pid)) => {
            info!("mercury is running (pid {pid})");
            0
        }
        // The window between a daemon taking the lock and writing its pid into it. Something is
        // running, and that is the question this verb was asked, so it answers yes without the
        // pid rather than waiting for one.
        //
        // `stop` treats the same state as a failure, because a signal needs a pid and there is
        // none. Neither is wrong: they are asking the lock different questions.
        Ok(Held::Unnamed) => {
            info!("mercury is running (it has just started and has not recorded its pid)");
            0
        }
        Err(e) => {
            warn!("mercury: {e}");
            1
        }
    }
}
```

`use freddie_single_instance::Held;` and `use crate::logging::Terminal;` join the imports `client.rs` already has.

## `mercury logs`

The file records `debug` always, and the daemon writes a record per keystroke, so an unfiltered follow is a firehose that hides the thing you were watching for. The file keeps everything; this chooses what to show.

```rust
/// Where `tail` lives. Absolute, so `PATH` cannot point this at something else.
const TAIL: &str = "/usr/bin/tail";

/// How much of the existing log to show before following.
const TAIL_LINES: &str = "50";

/// The level field the `fmt` layer writes, surrounded as it appears in a record.
///
/// Reading the text is what filtering a formatted log costs. The alternative is a machine format,
/// which would break the thing the file is for: `CLAUDE.md` sends a person, or an agent, to read
/// it directly.
const DEBUG_RECORD: &str = " DEBUG ";

/// Follow the log file: show the tail of what is there, then whatever arrives.
///
/// `tail -F` rather than a follower of our own. It waits for a file that does not exist yet, which
/// is the first run on a machine before anything has been logged, and it reopens by name if the
/// file is replaced. Verified on macOS 25.5.0 against a path created a second after `tail`
/// started: the lines appear.
///
/// Its stdout is piped rather than inherited, so each line can be dropped or shown. Its stderr and
/// its process group are inherited, so Ctrl-C reaches it and ends the follow. That is the whole
/// reason `mercury-start.md` puts the daemon in a group of its own.
///
/// Lines are written straight to stdout rather than traced: they are already records, out of the
/// file this is following, and tracing them would put them back into it.
pub(crate) fn logs(args: &LogsArgs) -> i32 {
    // `init` returns where it put the log, which is the file to follow.
    let path = logging::init(Terminal::Client);
    info!("mercury: following {}", path.display());

    let mut tail = match std::process::Command::new(TAIL)
        .args(["-n", TAIL_LINES])
        .arg("-F")
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("mercury: could not run {TAIL}: {e}");
            return 1;
        }
    };

    let Some(stdout) = tail.stdout.take() else {
        warn!("mercury: {TAIL} gave no stdout to read");
        return 1;
    };
    let mut out = std::io::stdout().lock();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if args.debug || !line.contains(DEBUG_RECORD) {
            // A closed stdout is the pipeline this was feeding going away, which ends the follow
            // rather than being worth a word about.
            if writeln!(out, "{line}").is_err() {
                break;
            }
        }
    }
    match tail.wait() {
        Ok(status) => i32::from(!status.success()),
        Err(e) => {
            warn!("mercury: {TAIL} could not be waited on: {e}");
            1
        }
    }
}
```

```rust
/// What `mercury logs` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct LogsArgs {
    /// Show `DEBUG` records too, which is every key the daemon dispatched.
    ///
    /// The file always has them. This is about what reaches the terminal.
    #[arg(long)]
    pub debug: bool,
}
```

Imports gained in `client.rs`: `std::io::{BufRead, BufReader, Write}` and `std::process::Stdio`.

`logging.rs` needs nothing new. Every verb initializes logging now that the terminal is a layer, and `init` already returns the path it logged to, which is the path `logs` follows.

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
    Logs(LogsArgs),
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
        Some(Verb::Logs(args)) => client::logs(&args),
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
    assert!(matches!(parse(&["logs"]).verb, Some(Verb::Logs(_))));
}

#[test]
fn logs_hides_debug_by_default() {
    let Some(Verb::Logs(args)) = parse(&["logs"]).verb else {
        panic!("the logs verb parses to Verb::Logs");
    };
    assert!(!args.debug);
}

#[test]
fn logs_takes_debug() {
    let Some(Verb::Logs(args)) = parse(&["logs", "--debug"]).verb else {
        panic!("the logs verb parses to Verb::Logs");
    };
    assert!(args.debug);
}
```

## Verifying

- `mercury status` with nothing running prints "mercury is not running" and exits 1.
- `mercury daemon` in another pane, then `mercury status` prints that pane's pid and exits 0, and the pid matches the one `mercury stop` reports when it ends it.
- `mercury status` says its line on the terminal and the same words appear in the log, stamped with the client's pid.
- `mercury logs` shows the tail of the log and then follows it: switching layers in the daemon makes `dispatch` lines appear. Ctrl-C returns to the shell and leaves the daemon running.
- `mercury logs` shows no `DEBUG` records while the daemon is logging a record per keystroke; `mercury logs --debug` shows them.
- `mercury logs | head -5` exits rather than failing when `head` closes the pipe.
- `mercury logs` on a machine that has never run mercury waits, printing nothing, and starts printing when a daemon first writes.
