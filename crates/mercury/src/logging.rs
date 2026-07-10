//! Where mercury's tracing goes.
//!
//! Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
//! the record of a run survives however quiet the terminal was asked to be. The
//! terminal shows whatever `LOG_LEVEL` asks for, defaulting to `info`.

use std::path::PathBuf;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// The log file, written under [`log_dir`].
const LOG_FILE: &str = "mercury.log";

/// The environment variable that sets the terminal's log level.
const LOG_LEVEL_ENV: &str = "LOG_LEVEL";

/// What the log file records, always. Deliberately not tied to [`LOG_LEVEL_ENV`]:
/// the file is the record of what happened, so quieting the terminal must never
/// quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Where the log file lives: the macOS per-user log directory, or the current
/// directory when `HOME` is unset.
fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Logs/mercury"),
    )
}

/// Send tracing to the log file and the terminal, and return the file's path.
pub fn init() -> PathBuf {
    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("mercury: could not create {}: {e}", dir.display());
    }

    let file = fmt::layer()
        .with_writer(tracing_appender::rolling::never(&dir, LOG_FILE))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);

    let terminal = fmt::layer().with_writer(std::io::stderr).with_filter(
        EnvFilter::try_from_env(LOG_LEVEL_ENV).unwrap_or_else(|_| EnvFilter::new("info")),
    );

    tracing_subscriber::registry()
        .with(file)
        .with(terminal)
        .init();
    dir.join(LOG_FILE)
}
