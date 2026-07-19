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

use tracing_subscriber::filter::LevelFilter;
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
/// `directives` is a `tracing_subscriber` filter string, so `info` and
/// `mercury=debug,bind=warn` are both accepted. One that does not parse falls back
/// to `info` and says so on the terminal, since the alternative is a run with no
/// logging at all.
pub fn init(directives: &str) -> PathBuf {
    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("mercury: could not create {}: {e}", dir.display());
    }

    let file = fmt::layer()
        .with_writer(WithPid(tracing_appender::rolling::never(&dir, LOG_FILE)))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);

    let terminal = fmt::layer().with_writer(std::io::stderr).with_filter(
        EnvFilter::try_new(directives).unwrap_or_else(|e| {
            eprintln!("mercury: {directives:?} is not a log filter ({e}); using info");
            EnvFilter::new("info")
        }),
    );

    tracing_subscriber::registry()
        .with(file)
        .with(terminal)
        .init();
    dir.join(LOG_FILE)
}
