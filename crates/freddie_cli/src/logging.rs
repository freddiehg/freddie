//! Where a daemon's tracing goes, and where the verbs that talk to it write.
//!
//! Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
//! the record of a run survives however quiet the terminal was asked to be. The
//! terminal shows whatever [`LOG_LEVEL`] asks for, defaulting to `info`.
//!
//! One file per daemon, with as many writers as there are processes talking to it, so
//! every record it takes is stamped with the pid of the process that wrote it.

use std::cell::RefCell;
use std::io;
use std::sync::OnceLock;

use tracing::{Level, warn};

use crate::Instance;
use tracing_subscriber::filter::{self, LevelFilter};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// What the terminal shows when [`LOG_LEVEL`] says nothing. Shared with `logs`, so a
/// daemon's own terminal and a follower of its file show the same records by default.
pub(crate) const DEFAULT_LOG_LEVEL: &str = "info";

/// What the log file records, always. Deliberately not tied to the terminal's
/// filter: the file is the record of what happened, so quieting the terminal must
/// never quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Wraps a writer so every record carries the pid of the process that wrote it.
///
/// The file has as many writers as there are processes on one daemon, and a record says
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
    ///
    /// A record that does not start with `{` is written through untouched. The formatter always
    /// produces one that does, and a record that somehow did not would be destroyed by having its
    /// first byte replaced.
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        STAMPED.with_borrow_mut(|line| {
            line.clear();
            match record.strip_prefix(b"{") {
                Some(rest) => {
                    line.extend_from_slice(stamp().as_bytes());
                    line.extend_from_slice(rest);
                }
                None => line.extend_from_slice(record),
            }
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
    /// A daemon may log at `debug` on every event it takes, so a buffer per record would
    /// allocate there. Thread-local because a daemon writes from more than one thread.
    static STAMPED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// This process's stamp: the opening brace of the record's object and the pid inside it, so
/// splicing it in front of a record whose own opening brace has been taken off puts the pid first.
///
/// Built once. A pid does not change under a running process.
fn stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| format!("{{\"pid\":{},", std::process::id()))
}

/// Send tracing to this daemon's log file and to this process's terminal.
///
/// A daemon reads its directives from [`LOG_LEVEL`], a `tracing_subscriber` filter
/// string, so `info` and `warn,some_crate=debug` are both accepted. One that does not
/// parse falls back to [`DEFAULT_LOG_LEVEL`] and says so, since the alternative is a run
/// with no logging.
pub(crate) fn init(instance: &Instance, terminal: Terminal) {
    let dir = instance.log_dir();
    // Held rather than said: there is no subscriber yet to say them to, and a setup
    // failure belongs in the file as much as anything else does.
    let mut setup = Vec::new();
    if let Err(e) = std::fs::create_dir_all(dir) {
        setup.push(format!("could not create {}: {e}", dir.display()));
    }

    let file = fmt::layer()
        .json()
        // One flat object per record: `flatten_event` lifts the event's own fields up beside
        // `timestamp` and `target` rather than nesting them under `fields`, and dropping the span
        // keys leaves nothing else in the object.
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(WithPid(tracing_appender::rolling::never(
            dir,
            instance.log_file_name(),
        )))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);

    let registry = tracing_subscriber::registry().with(file);
    match terminal {
        Terminal::Daemon => {
            let directives =
                std::env::var(LOG_LEVEL).unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_owned());
            let filter = EnvFilter::try_new(&directives).unwrap_or_else(|e| {
                setup.push(format!(
                    "{LOG_LEVEL}={directives:?} is not a log filter ({e}); using {DEFAULT_LOG_LEVEL}"
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
        warn!("{}: {problem}", instance.display_name());
    }
    log_panics();
}

/// Send panics through `tracing` instead of straight to stderr.
///
/// The default hook prints and nothing else, which for a detached daemon means the one
/// record of why it died goes to a terminal nobody is attached to. Routed through
/// `error!`, it reaches the log file like everything else, and a client verb still shows
/// it because `error!` is what goes to a client's stderr.
///
/// The backtrace follows `RUST_BACKTRACE`, the way the default hook's does.
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

#[cfg(test)]
mod tests {
    use super::{PidStamped, WithPid};
    use std::io::Write;

    // The real json layer, through the real pid stamp, produces a line shaped the way
    // `client::Record` reads it. This pins the record shape against a
    // `tracing-subscriber` that changes its JSON: the event's own fields sit at the top level, and
    // they keep the order they were logged, which is what puts `event` before `effects`.
    #[test]
    fn the_file_layer_writes_a_flat_record_in_logged_order() {
        let path =
            std::env::temp_dir().join(format!("freddie_cli_seam_{}.log", std::process::id()));
        let file = std::fs::File::create(&path).expect("a temp log file");
        let subscriber = tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .with_ansi(false)
            .with_writer(WithPid(file))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                event = "Key(KeyR)",
                effects = "[]",
                state = "Mercury { .. }",
                "dispatch"
            );
        });

        let line = std::fs::read(&path).expect("the temp log file");
        std::fs::remove_file(&path).ok();
        let record: serde_json::Value = serde_json::from_slice(&line).expect("a record");
        assert_eq!(record["pid"], serde_json::json!(std::process::id()));
        assert!(record["timestamp"].is_string());
        assert_eq!(record["level"], serde_json::json!("INFO"));
        assert!(record["target"].is_string());
        // Flat, not nested under `fields`.
        assert_eq!(record["message"], serde_json::json!("dispatch"));
        assert_eq!(record["event"], serde_json::json!("Key(KeyR)"));
        assert_eq!(record["state"], serde_json::json!("Mercury { .. }"));

        let keys: Vec<&String> = record.as_object().expect("an object").keys().collect();
        let at = |k: &str| keys.iter().position(|key| *key == k).expect("the key");
        assert!(at("event") < at("effects"), "event should precede effects");
    }

    // A record with no leading brace would be destroyed by having its first byte replaced, so it is
    // written through untouched instead.
    #[test]
    fn a_line_that_is_not_an_object_is_written_through() {
        let mut written = Vec::new();
        PidStamped(&mut written)
            .write_all(b"a stray line")
            .expect("writing to a Vec");
        assert_eq!(written, b"a stray line");
    }
}

/// The environment variable a daemon reads its terminal filter from.
///
/// Not a flag: the only invocation with a terminal to filter is one a person typed in
/// front of, and `daemon` is hidden, spawned by `start` with its output at /dev/null, and
/// run by launchd with no terminal at all. A variable serves the one case a flag would.
pub const LOG_LEVEL: &str = "LOG_LEVEL";

/// What the terminal shows, which is not the same for the process that is the daemon
/// and the processes that talk to it.
#[derive(Clone, Copy)]
pub(crate) enum Terminal {
    /// The daemon. Its terminal is a view of its log: every record in full, filtered by
    /// what [`LOG_LEVEL`] asked for.
    Daemon,
    /// A client verb. Its terminal is its output: `INFO` is the answer and goes to
    /// stdout, `WARN` and above are problems and go to stderr, and both are shown as the
    /// bare message.
    Client,
}

/// The layers a client verb shows on its terminal.
///
/// `without_time`, `with_level(false)`, `with_target(false)`: a verb's output is the
/// thing the user asked for, and a timestamp and a level in front of it would make
/// `status` unusable in a pipeline. The file layer keeps all of it, so nothing is lost
/// by leaving it off here.
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
