# bounding the log file

A daemon's log file grows forever. It has no ceiling, nothing rotates it, and the file layer's filter is a constant, so there is no way to ask it for less. Measured on a mercury log covering thirteen days: 443 MB, 1,923,847 lines, about 34 MB a day.

Two things are in that file, and only one of them is worth its size.

The dispatch record is one line per event carrying the event, the effects it produced, and the resulting state. That is what makes a run reconstructable, and it is a third of the lines.

The per-key `debug` records are three sites that each write a line per keystroke: `post` in `freddie_keyboard`, `tapped` and `emitted` in mercury's daemon. About 1.1M of the 1.92M lines. A dispatch record already carries the key it was handed and the `Emit` effects it produced, so these are the second and third telling of what the file already says.

The content matters as much as the volume. A record of which keys were pressed, in order, with timestamps, is what was typed. It sits in `~/Library/Logs`, readable by anything running as the user, and it is currently kept forever.

So: give the per-key records a target that can be turned off, make the file's filter something a run can set, and give the file a ceiling with a fixed number of files behind it.

## Change 1: the keystroke records name themselves, and the file mutes them

### The target

`crates/freddie_keys/src/lib.rs`, added:

```rust
/// The tracing target of a record written once per keystroke.
///
/// Shared by every site that writes one, in whichever crate, so that `keystroke=off` in a filter
/// turns off all of them and not whichever the person writing the filter remembered. The target
/// column of such a record reads `keystroke` rather than the module it came from; the message
/// (`post`, `tapped`, `emitted`) is what tells them apart.
pub const KEYSTROKE_TARGET: &str = "keystroke";
```

`freddie_keys` because it is the one crate both `freddie_keyboard` and `mercury` already depend on.

### The sites

`crates/freddie_keyboard/src/sys/macos.rs`, before:

```rust
    tracing::debug!(
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        kept_from_source = %format!("{:#010x}", untouched.bits()),
        kind = ?event.get_type(),
        "post"
    );
```

after:

```rust
    tracing::debug!(
        target: freddie_keys::KEYSTROKE_TARGET,
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        kept_from_source = %format!("{:#010x}", untouched.bits()),
        kind = ?event.get_type(),
        "post"
    );
```

`crates/mercury/src/daemon.rs`, before:

```rust
            Ok(()) => debug!(?key, ?flags, "tapped"),
```

after:

```rust
            Ok(()) => debug!(target: freddie_keys::KEYSTROKE_TARGET, ?key, ?flags, "tapped"),
```

`crates/mercury/src/daemon.rs`, before:

```rust
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
```

after:

```rust
            Ok(()) => debug!(
                target: freddie_keys::KEYSTROKE_TARGET,
                key = ?ke.key,
                press = ?ke.press,
                "emitted"
            ),
```

### The file's filter

Both variables do the same thing with what they are given, so the fallback is written once.

`crates/freddie_cli/src/logging.rs`, added:

```rust
/// The filter `variable` asks for, or `fallback` when it says nothing or says something that is
/// not a filter. A run with an unparseable directive still logs, and the file says what was
/// wrong with it.
fn filter_from(variable: &str, fallback: &str, setup: &mut Vec<String>) -> EnvFilter {
    let directives = std::env::var(variable).unwrap_or_else(|_| fallback.to_owned());
    EnvFilter::try_new(&directives).unwrap_or_else(|e| {
        setup.push(format!(
            "{variable}={directives:?} is not a log filter ({e}); using {fallback}"
        ));
        EnvFilter::new(fallback)
    })
}
```

`crates/freddie_cli/src/logging.rs`, before:

```rust
/// What the log file records, always. Deliberately not tied to the terminal's
/// filter: the file is the record of what happened, so quieting the terminal must
/// never quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;
```

after:

```rust
/// What the log file records when [`LOG_FILE_LEVEL`] says nothing.
///
/// `debug` for everything, less the target that writes a line per keystroke. A dispatch record
/// already carries the key it was handed and the keys it emitted, so what `keystroke=off` drops
/// is a retelling and not the record of what happened.
const DEFAULT_FILE_LEVEL: &str = "debug,keystroke=off";
```

`crates/freddie_cli/src/logging.rs`, added beside [`LOG_LEVEL`]:

```rust
/// The environment variable the log file reads its filter from.
///
/// Separate from [`LOG_LEVEL`], which is the terminal's alone: quieting a terminal must not
/// quiet the file, so one variable cannot serve both. `LOG_FILE_LEVEL=debug` is what turns the
/// keystroke records back on for a run that is debugging the keyboard path.
pub const LOG_FILE_LEVEL: &str = "LOG_FILE_LEVEL";
```

`init`, before:

```rust
    let file = fmt::layer()
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
```

after:

```rust
    let file = fmt::layer()
        .with_writer(WithPid(tracing_appender::rolling::never(
            dir,
            instance.log_file_name(),
        )))
        .with_ansi(false)
        .with_filter(filter_from(
            LOG_FILE_LEVEL,
            DEFAULT_FILE_LEVEL,
            &mut setup,
        ));

    let registry = tracing_subscriber::registry().with(file);
    match terminal {
        Terminal::Daemon => registry
            .with(
                fmt::layer()
                    .with_writer(io::stderr)
                    .with_filter(filter_from(LOG_LEVEL, DEFAULT_LOG_LEVEL, &mut setup)),
            )
            .init(),
        Terminal::Client => registry.with(client_terminal()).init(),
    }
```

`LevelFilter` stays imported: `client_terminal` still uses it.

### What the docs say

`CLAUDE.md`, in `## Logs`, before:

```
The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards.
```

after:

```
The file records down to `debug` whatever the terminal is set to, less the keystroke records: `post`, `tapped`, and `emitted` write one line per key, and the dispatch record already carries the key it was handed and the keys it emitted. `LOG_FILE_LEVEL` is what changes that, so `LOG_FILE_LEVEL=debug` turns them back on for a run that is debugging the keyboard path.
```

`README.md`, in `## mercury logs`, before:

```
`mercury` writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and always down to `debug` whatever the terminal was asked for.
```

after:

```
`mercury` writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and down to `debug` whatever the terminal was asked for. The per-keystroke records are off by default; `LOG_FILE_LEVEL=debug` turns them on.
```

## Change 2: the file rolls at a ceiling, and only a daemon rolls it

### The bounded file

`crates/freddie_cli/src/logging.rs`, added:

```rust
/// What one log file may reach before it rolls.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// How many rolled files are kept behind the live one, so the whole log is at most
/// `MAX_BYTES * (KEEP + 1)`.
const KEEP: usize = 3;

/// A log file with a ceiling.
///
/// It appends until the file reaches [`MAX_BYTES`], then rolls: `mercury.log.2` becomes
/// `mercury.log.3`, `mercury.log.1` becomes `mercury.log.2`, `mercury.log` becomes
/// `mercury.log.1`, and whatever was in `mercury.log.{KEEP}` is gone. The live file keeps its
/// name across all of it, so `Instance::log_file` and the `tail -F` under `logs` go on naming
/// one path.
///
/// The size is read from the file rather than counted here, because this is not the only writer:
/// a client verb appends to the same file knowing nothing about what a daemon has written.
pub(crate) struct BoundedLog {
    path: PathBuf,
    file: Mutex<File>,
}

impl BoundedLog {
    /// Open `path` for appending, creating it and its directory entry if it is not there.
    ///
    /// # Errors
    ///
    /// Whatever opening the file failed with.
    pub(crate) fn open(path: PathBuf) -> io::Result<Self> {
        let file = Mutex::new(append_to(&path)?);
        Ok(Self { path, file })
    }

    /// Shift the numbered files up by one, put the live file at `.1`, and open a new one in its
    /// place.
    ///
    /// A rename over a file that is not there is the expected case until [`KEEP`] rolls have
    /// happened, so its failure is the answer rather than a problem. `rename` replaces its
    /// destination, which is what drops the oldest.
    fn roll(&self, file: &mut File) -> io::Result<()> {
        for n in (1..KEEP).rev() {
            drop(std::fs::rename(self.numbered(n), self.numbered(n + 1)));
        }
        std::fs::rename(&self.path, self.numbered(1))?;
        *file = append_to(&self.path)?;
        Ok(())
    }

    /// `mercury.log.2` for 2.
    fn numbered(&self, n: usize) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }
}

/// Open a file to append to, making it if it is not there.
fn append_to(path: &Path) -> io::Result<File> {
    OpenOptions::new().append(true).create(true).open(path)
}

impl<'a> MakeWriter<'a> for BoundedLog {
    type Writer = &'a Self;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

impl io::Write for &BoundedLog {
    /// One `write_all` per record, under the lock, so two of a daemon's threads cannot interleave
    /// halves of a line and a roll cannot happen in the middle of one.
    ///
    /// A file whose length cannot be read is treated as under the ceiling: the record is worth
    /// more than the roll, and the next write asks again.
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if file.metadata().is_ok_and(|m| m.len() >= MAX_BYTES) {
            self.roll(&mut file)?;
        }
        file.write_all(record)?;
        Ok(record.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}
```

Imports this adds to the file:

```rust
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
```

### Who may roll

`crates/freddie_cli/src/logging.rs`, added:

```rust
/// Where a process's records go, which is not the same for a daemon and for the verbs that talk
/// to it.
///
/// Rolling belongs to the daemon alone. A verb is one short invocation and there may be several
/// at once; if each rolled, two would race over the renames while a third wrote through them. So
/// a client is handed a writer that can only append, and that is the type it has rather than a
/// rule a call site keeps.
enum LogFile {
    /// The daemon's, which rolls at [`MAX_BYTES`].
    Rolling(BoundedLog),
    /// A client verb's, which appends to whatever file is there.
    Appending(RollingFileAppender),
}

/// The writer one of them makes.
enum LogWriter<'a> {
    Rolling(&'a BoundedLog),
    Appending(RollingWriter<'a>),
}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = LogWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        match self {
            Self::Rolling(log) => LogWriter::Rolling(log.make_writer()),
            Self::Appending(appender) => LogWriter::Appending(appender.make_writer()),
        }
    }
}

impl io::Write for LogWriter<'_> {
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        match self {
            Self::Rolling(w) => w.write(record),
            Self::Appending(w) => w.write(record),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Rolling(w) => w.flush(),
            Self::Appending(w) => w.flush(),
        }
    }
}
```

Imports this adds:

```rust
use tracing_appender::rolling::{RollingFileAppender, RollingWriter};
```

### Choosing one

`init`, before:

```rust
    let file = fmt::layer()
        .with_writer(WithPid(tracing_appender::rolling::never(
            dir,
            instance.log_file_name(),
        )))
        .with_ansi(false)
        .with_filter(filter_from(
            LOG_FILE_LEVEL,
            DEFAULT_FILE_LEVEL,
            &mut setup,
        ));
```

after:

```rust
    // A daemon that cannot open its log for rolling still logs, to the same path, unbounded. The
    // file saying why is worth more than the ceiling it did not get.
    let appending = || LogFile::Appending(tracing_appender::rolling::never(dir, instance.log_file_name()));
    let file = match terminal {
        Terminal::Daemon => match BoundedLog::open(instance.log_file()) {
            Ok(log) => LogFile::Rolling(log),
            Err(e) => {
                setup.push(format!(
                    "could not open {} to roll it ({e}); it will grow without bound",
                    instance.log_file().display()
                ));
                appending()
            }
        },
        Terminal::Client => appending(),
    };

    let file = fmt::layer()
        .with_writer(WithPid(file))
        .with_ansi(false)
        .with_filter(filter_from(
            LOG_FILE_LEVEL,
            DEFAULT_FILE_LEVEL,
            &mut setup,
        ));
```

`Terminal` is `Copy`, so the match below it still has one.

### What a roll costs a client

A client verb that is writing when the daemon renames the file has the old inode open, so its record lands in `mercury.log.1` rather than `mercury.log`. It is one invocation's few lines, in the file one back, and the window is the length of a rename. Nothing is lost and nothing is torn.

### Tests

`crates/freddie_cli/src/logging.rs`, added:

```rust
#[cfg(test)]
mod tests {
    use super::{BoundedLog, KEEP, MAX_BYTES};
    use std::io::Write;

    /// A file under the ceiling is appended to and nothing is renamed.
    #[test]
    fn writes_below_the_ceiling_do_not_roll() { /* ... */ }

    /// Crossing the ceiling moves the live file to `.1` and starts a new one at the same path.
    #[test]
    fn crossing_the_ceiling_rolls_to_one() { /* ... */ }

    /// `KEEP` rolls fill `.1` through `.{KEEP}`; the next drops what was oldest and no more files
    /// than that exist.
    #[test]
    fn the_oldest_is_dropped_and_the_count_holds() { /* ... */ }

    /// The live file is at the path it was opened with after every roll, since that is the path
    /// `logs` follows.
    #[test]
    fn the_live_file_keeps_its_name() { /* ... */ }
}
```

Each builds a `BoundedLog` in a temporary directory with the constants as they are, writing `MAX_BYTES`-sized records to cross the ceiling in one write.

### What the docs say

`CLAUDE.md`, in `## Logs`, before:

```
mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.
```

after:

```
mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.

The file has a ceiling. At 32 MiB the daemon rolls it: `mercury.log` becomes `mercury.log.1`, each numbered file shifts up one, and the fourth is dropped. So the log is at most 128 MiB and reaches back about a week, and the live file is always `mercury.log`. A client verb appends but never rolls, because one daemon is the only writer that is always there.
```

`README.md`, in `## mercury logs`, appended:

```
The file rolls at 32 MiB into `mercury.log.1` through `mercury.log.3`, so it holds about a week and never more than 128 MiB.
```
