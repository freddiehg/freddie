//! Where mercury's tracing goes.
//!
//! Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
//! the record of a run survives however quiet the terminal was asked to be. The
//! terminal shows whatever `--log-level` asks for, defaulting to `info`.
//!
//! The file has one writer per running mercury process, so every record it takes is
//! stamped with the pid of the process that wrote it.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use tracing::{Level, warn};

use crate::cli::DEFAULT_LOG_LEVEL;
use tracing_subscriber::filter::{self, LevelFilter};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// The log file, written under [`log_dir`].
const LOG_FILE: &str = "mercury.log";

/// What the log file records, always. Deliberately not tied to the terminal's
/// filter: the file is the record of what happened, so quieting the terminal must
/// never quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Wraps a writer so every record carries the pid of the process that wrote it.
///
/// The file has as many writers as there are mercury processes, and a record says
/// which module emitted it but not which process. Two clients running at once are
/// otherwise the same line.
struct WithPid<W>(W);

impl<'a, W: MakeWriter<'a>> MakeWriter<'a> for WithPid<W> {
    type Writer = PidStamped<W::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        PidStamped(self.0.make_writer())
    }
}

/// The stamped writer. `fmt` calls `write` once per record, so this stamps once per
/// record.
struct PidStamped<W>(W);

impl<W: io::Write> io::Write for PidStamped<W> {
    /// One `write_all` for the stamp and the record together.
    ///
    /// Two calls would be two appends, and another process may append between them,
    /// which would leave a stamp attached to a stranger's record. Building the line
    /// first is what keeps a record whole against the other writers this exists for.
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

thread_local! {
    /// The line being assembled, reused across records.
    ///
    /// mercury logs every key at `debug`, so this runs on every keystroke and a buffer
    /// per record would allocate there. Thread-local because the daemon writes from
    /// three threads.
    static STAMPED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// This process's stamp, built once. A pid does not change under a running process.
fn stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| format!("pid={} ", std::process::id()))
}

/// Where the log file lives: the macOS per-user log directory, or the current
/// directory when `HOME` is unset.
fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Logs/mercury"),
    )
}

/// Send tracing to the log file and the terminal, and return the file's path.
///
/// A daemon's directives are a `tracing_subscriber` filter string, so `info` and
/// `mercury=debug,bind=warn` are both accepted. One that does not parse falls back to
/// [`DEFAULT_LOG_LEVEL`] and says so, since the alternative is a run with no logging.
pub fn init(terminal: &Terminal<'_>) -> PathBuf {
    let dir = log_dir();
    // Held rather than said: there is no subscriber yet to say them to, and a setup
    // failure belongs in the file as much as anything else does.
    let mut setup = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        setup.push(format!("could not create {}: {e}", dir.display()));
    }

    let file = fmt::layer()
        .with_writer(WithPid(tracing_appender::rolling::never(&dir, LOG_FILE)))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);

    let registry = tracing_subscriber::registry().with(file);
    match terminal {
        Terminal::Daemon(LogLevel(directives)) => {
            let filter = EnvFilter::try_new(*directives).unwrap_or_else(|e| {
                setup.push(format!(
                    "{directives:?} is not a log filter ({e}); using {DEFAULT_LOG_LEVEL}"
                ));
                EnvFilter::new(DEFAULT_LOG_LEVEL)
            });
            registry
                .with(fmt::layer().with_writer(io::stderr).with_filter(filter))
                .init();
        }
        Terminal::Client => registry.with(client_terminal()).init(),
    }

    // The subscriber exists now, so anything held above can finally be said.
    for problem in setup {
        warn!("mercury: {problem}");
    }
    dir.join(LOG_FILE)
}

/// What the terminal shows, which is not the same for the process that is the daemon
/// and the processes that talk to it.
pub enum Terminal<'a> {
    /// The daemon. Its terminal is a view of its log: every record in full, filtered by
    /// what `--log-level` asked for.
    Daemon(LogLevel<'a>),
    /// A client verb. Its terminal is its output: `INFO` is the answer and goes to
    /// stdout, `WARN` and above are problems and go to stderr, and both are shown as the
    /// bare message.
    Client,
}

/// A `tracing_subscriber` filter directive, such as `info` or `mercury=debug,bind=warn`.
pub struct LogLevel<'a>(pub &'a str);

/// The layers a client verb shows on its terminal.
///
/// `without_time`, `with_level(false)`, `with_target(false)`: a verb's output is the
/// thing the user asked for, and a timestamp and a level in front of it would make
/// `mercury status` unusable in a pipeline. The file layer keeps all of it, so nothing
/// is lost by leaving it off here.
///
/// Two layers rather than one, because a result belongs on stdout and a problem on
/// stderr, and a layer has one writer. `INFO` exactly, rather than `INFO` and above, so
/// the split is total: no record reaches both.
fn client_terminal<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let results = fmt::layer()
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_ansi(false)
        .with_writer(io::stdout)
        .with_filter(filter::filter_fn(|meta| *meta.level() == Level::INFO));
    let problems = fmt::layer()
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_ansi(false)
        .with_writer(io::stderr)
        .with_filter(LevelFilter::WARN);
    results.and_then(problems)
}
