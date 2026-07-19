# one log, many writers, nothing printed

`~/Library/Logs/mercury/mercury.log` used to have one writer. Since `mercury stop`, every client invocation appends to it too, and the records interleave with the daemon's:

```
INFO mercury::client: signalling the daemon pid=19708 signal=Terminate
INFO mercury::daemon: SIGTERM: quitting
INFO mercury::daemon: kill: exiting
INFO mercury::client: the daemon released the lock pid=19708
```

That is the interleaving we want: the file is one account of a shutdown rather than two half-stories. Two things are missing from it. A record does not say which process wrote it, and the ten things mercury says through `println!` and `eprintln!` do not appear in it at all.

Both are fixed here. Afterwards the log is the whole of what mercury said, and every line of it names its speaker.

## Change 1: every record names its writer

### The pid on a client line means something else

`pid=19708` above is the daemon being signalled, written by a client whose own pid is something else. Stamping records with the writer's pid would put two different `pid=` on one line meaning two different things.

So the field is renamed where it names a subject rather than a writer. In `client.rs`:

```rust
info!(daemon = %pid, ?signal, "signalling the daemon");
warn!(daemon = %pid, %error, "could not signal the daemon");
info!(daemon = %pid, "the daemon released the lock");
warn!(daemon = %pid, timeout = ?STOP_TIMEOUT, "the daemon still holds the lock");
```

After which `pid=` on a record is always the writer, and `daemon=` is always the process being talked about.

### Stamping the file

```rust
/// Wraps a writer so every record carries the pid of the process that wrote it.
///
/// The file has as many writers as there are mercury processes, and a record says which module
/// emitted it but not which process. Two clients running at once are otherwise the same line.
struct WithPid<W>(W);

impl<'a, W: MakeWriter<'a>> MakeWriter<'a> for WithPid<W> {
    type Writer = PidStamped<W::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        PidStamped(self.0.make_writer())
    }
}

/// The stamped writer. `fmt` calls `write` once per record, so this stamps once per record.
struct PidStamped<W>(W);

impl<W: io::Write> io::Write for PidStamped<W> {
    /// One `write_all` for the stamp and the record together.
    ///
    /// Two calls would be two appends, and another process may append between them, which would
    /// leave a stamp attached to a stranger's record. Building the line first is what keeps a
    /// record whole against the other writers this exists for.
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        STAMPED.with_borrow_mut(|line| {
            line.clear();
            line.extend_from_slice(stamp().as_bytes());
            line.extend_from_slice(record);
            self.0.write_all(line)
        })?;
        Ok(record.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The line being assembled, reused across records.
///
/// mercury logs every key at `debug`, so the file path runs on every keystroke; a buffer per
/// record would allocate there. Thread-local because the daemon writes from three threads.
thread_local! {
    static STAMPED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// This process's stamp, built once. A pid does not change under a running process.
fn stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| format!("pid={} ", std::process::id()))
}
```

The file layer wraps its appender in it:

```diff
     let file = fmt::layer()
-        .with_writer(tracing_appender::rolling::never(&dir, LOG_FILE))
+        .with_writer(WithPid(tracing_appender::rolling::never(&dir, LOG_FILE)))
         .with_ansi(false)
         .with_filter(FILE_LEVEL);
```

The terminal is left alone. It shows one process's own output, so a pid there is a column of the same number on every line.

Imports gained: `std::cell::RefCell`, `std::io`, `std::sync::OnceLock`, `tracing_subscriber::fmt::MakeWriter`.

Verified on the pinned 1.96.0 against a `MakeWriter` of this shape: exactly one stamp per record, including a record longer than a formatting buffer and a record emitted from a second thread.

## Change 2: nothing is printed

`println!` and `eprintln!` write where no subscriber can see them, so everything mercury says on a terminal today is missing from the file that is supposed to be the record of the run. Every one of them becomes a tracing event, and the terminal becomes a layer like any other.

Six of the ten are already redundant: `daemon.rs` pairs each of its `eprintln!`s with an `error!` or `warn!` carrying the same thing, because the terminal could not otherwise see it. Those lose the print and keep the event.

```diff
     if let Err(e) = freddie_windows::init() {
-        eprintln!("windows: {e}");
         error!(error = %e, "window placement unavailable");
     }
```

and the same at the lock, the menu bar, and the keyboard grab, with `daemon.rs`'s `println!("mercury: logging to ...")` becoming `info!(path = %log_path.display(), "logging")`.

### What a terminal shows depends on who is speaking

The daemon's terminal output is its log, timestamps and all, which is what it already is. A client's terminal output is the verb's answer, which must read as a bare line: `mercury stopped (pid 19708)`, not a formatted record.

Both are `fmt` layers; they differ in format, in writer, and in filter.

```rust
/// What the terminal shows, which is not the same for the process that is the daemon and the
/// processes that talk to it.
pub enum Terminal<'a> {
    /// The daemon. Its terminal is a view of its log: every record in full, filtered by what
    /// `--log-level` asked for.
    Daemon(LogLevel<'a>),
    /// A client verb. Its terminal is its output: `INFO` is the answer and goes to stdout, `WARN`
    /// and above are problems and go to stderr, and both are printed as the bare message.
    Client,
}

/// A `tracing_subscriber` filter directive, such as `info` or `mercury=debug,bind=warn`.
pub struct LogLevel<'a>(pub &'a str);
```

```rust
/// The layers a client verb shows on its terminal.
///
/// `without_time`, `with_level(false)`, `with_target(false)`: a verb's output is the thing the
/// user asked for, and a timestamp and a level in front of it would make `mercury status` unusable
/// in a pipeline. The file layer keeps all of it, so nothing is lost by leaving it off here.
///
/// Two layers rather than one, because a result belongs on stdout and a problem on stderr, and a
/// layer has one writer. `INFO` exactly, rather than `INFO` and above, so the split is total: no
/// record reaches both.
fn client_terminal<S>() -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let bare = || {
        fmt::layer()
            .without_time()
            .with_level(false)
            .with_target(false)
            .with_ansi(false)
    };
    let results = bare()
        .with_writer(std::io::stdout)
        .with_filter(filter::filter_fn(|meta| *meta.level() == Level::INFO));
    let problems = bare()
        .with_writer(std::io::stderr)
        .with_filter(LevelFilter::WARN);
    results.and_then(problems)
}
```

Verified on the pinned 1.96.0: this format writes `mercury stopped (pid 19708)\n` and nothing else. Fields are appended to the visible line, so a record whose message is the output carries none: the pid goes in the message, and the structured `daemon=` records inside `stop_daemon` are what the file gets.

`init` takes which one it is:

```rust
pub fn init(terminal: &Terminal<'_>) -> PathBuf
```

By reference: `init` matches on it and consumes nothing, which the workspace's `clippy::pedantic` requires.

with `daemon::run` passing `Terminal::Daemon(LogLevel(&args.log_level))` and each client verb passing `Terminal::Client`. `CLIENT_LOG_LEVEL` disappears: a client's terminal filter is the level split above, not a directive.

### A client's level is its audience

The split above makes the level decide who sees a record, so a client verb's levels mean something they did not mean before:

- `info!` is the verb's answer. It reaches stdout, and there is one per invocation.
- `warn!` and `error!` are the problem the user has to see. They reach stderr.
- `debug!` is what the verb did along the way. Only the file keeps it.

`stop_daemon`'s narration is all `debug!` for that reason. At `info!` it printed three lines where the verb had one thing to say:

```
signalling the daemon daemon=23922 signal=Terminate
the daemon released the lock daemon=23922
mercury stopped (pid 23922)
```

and its `warn!`s printed the failure twice, once as narration and once as `stop`'s own line.

```rust
        Ok(Some(pid)) => {
            info!("mercury stopped (pid {pid})");
            0
        }
        Ok(None) => {
            info!("mercury is not running");
            0
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            1
        }
```

`status` and `logs` from `mercury-status-and-logs.md` say their lines the same way. That doc's "read-only verbs leave no record" section goes: with the terminal as a layer, a verb that says anything says it into the file too, and the reason for the exception — that a `status` on a timer would fill the log — is answered by the file layer's own filter rather than by the verb staying silent.

### The two that cannot be traced yet

`logging.rs` reports its own setup failures before there is a subscriber to report them to. Rather than leaving them as prints, they are held until there is one:

```rust
pub fn init(terminal: &Terminal<'_>) -> PathBuf {
    let dir = log_dir();
    // Held rather than printed: there is no subscriber yet, and a setup failure belongs in the
    // file as much as anything else does.
    let mut setup = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        setup.push(format!("could not create {}: {e}", dir.display()));
    }
    // ... build and install the layers, with the filter fallback pushing onto `setup` too ...
    for problem in setup {
        warn!("{problem}");
    }
    log_path()
}
```

A directory that could not be created means the file layer has nothing to write to, so that warning reaches the terminal only. It is still said, and said in one place.

### The audit

`println!`, `eprintln!`, `print!`, `eprint!`, `dbg!`, and direct writes to `io::stdout`/`io::stderr` across every crate: ten, all of them in `crates/mercury/src`, none anywhere else and none in any test. So the rule after this change is total, and a new one is a diff away from being noticed.

Two things are not routed, and both are somebody else's output rather than mercury's.

clap writes `--help`, `--version`, and its parse errors itself and exits, without returning to any code of ours.

`tail`, under `mercury logs`, writes the log's own contents to the terminal it inherited. Those lines are records already — that is where they came from — so re-emitting them as records would append them to the file being followed, which would show them again. `logs` displays; it does not re-log. `mercury-status-and-logs.md` gives it a filter for choosing what to display, because the file keeps everything regardless.

## Change 3: every stop shows up, including the one that did nothing

`stop` traces what it did to a daemon it found. A `stop` that found nothing running returns `Ok(None)` without a word about having been asked.

The verb says it ran, before it looks for anything:

```rust
pub(crate) fn stop(args: &StopArgs) -> i32 {
    logging::init(Terminal::Client);
    info!(force = args.force, "stop requested");
```

`stop requested` carries a field, so it prints as `stop requested force=false` on the terminal. That is a record of an action rather than the verb's answer, so it wants `debug!` instead: the file keeps it, the terminal does not show it, and the answer remains the only thing a user sees.

## Verifying

- `mercury daemon` in one pane and `mercury stop` in another: every line in the log carries a `pid=`, the daemon's lines all carry one pid, the client's all carry another, and the client's `daemon=` field matches the daemon's own `pid=`.
- `mercury stop` prints exactly `mercury stopped (pid N)` on stdout, with nothing on stderr, and the same words appear in the log with a timestamp and a stamp.
- `mercury stop` with nothing running writes `stop requested` and `mercury is not running` to the file, and prints only the second.
- `mercury stop` against a daemon that will not go prints its line on stderr, so `mercury stop > /dev/null` still shows the failure.
- `mercury daemon` with a busy lock says so on the terminal exactly once, where it said it twice before.
- Two `mercury stop` invocations at once are told apart by their stamps.
