# `logs` hands the terminal to lnav

`mercury logs` renders records and follows the file, and that is all it does. There is no way to stop the scroll and read what just went past, no way to narrow to the records you care about, and no way to jump to the last error. Every one of those is a key press in [lnav](https://lnav.org), which reads JSON-lines logs natively, so `logs` stops being a renderer and becomes the thing that points lnav at the right file with the right format.

Two JSON assets are shipped in `freddie_cli` and written to disk on every `logs`: a log format that teaches lnav the record envelope, and a keymap that binds the keys this change is about. Nothing else is written, and `~/.lnav` is never touched: lnav takes the directory from `-I` on the command line.

lnav is a new dependency of the `logs` verb. It is a `freddie_*` crate's to state: `logs` requires `lnav` on `PATH` (`brew install lnav`), and says so when it is missing. Every other verb is unaffected.

The version this is written against is lnav 0.14.0. Two things below are version-sensitive and were checked against it rather than read out of the docs: `:prompt`, which is what a key press uses to open a command prompt prefilled without running it, and `lnav_views.paused`, whose stock toggle is broken and is corrected here. Everything else in this document was rendered from a live figaro log of 295,782 records, which lnav indexed in 0.27s and rendered headlessly in 1.4s, so the size these logs reach is not a reason to slice the file first.

## What the user does

`mercury logs` opens lnav on `~/Library/Logs/mercury/mercury.log`, at the bottom, following.

- `p` pauses and resumes. While paused the status line reads `‖ Paused` and new records queue up rather than scrolling past.
- `f` opens `:filter-in `, prefilled. Type a regex, press Enter, and only matching records remain. The status line reads `1 of 1 enabled` and `N Lines not shown`.
- `F` opens `:filter-out `, prefilled. Matching records go.
- `Ctrl-F` toggles every filter off and back on, so the full log is one key away.
- `Tab` opens the filter panel, where filters are edited and deleted.
- `/` searches, `n` and `N` walk the hits. Search highlights and jumps; it does not hide anything.
- `e` and `E` jump to the next and previous error, `w` and `W` to the next and previous warning.
- `x` shows and hides the fields the format hides, which is `state`.
- `i` is a histogram of records over time, `;` is a SQL prompt over the log (`;SELECT * FROM freddie_log WHERE duration_us > 1000`).
- `q` quits.

lnav keeps a session per file, so the filters and the position from the last `mercury logs` are restored by the next one, and it says so on the status line. `Ctrl-R` resets the session and shows the whole log again.

`--level`, `--include-state`, and `--json` keep their meanings. `--level` and `--include-state` become lnav commands on the command line; `--json` writes the records as stored and follows, as it does now. `--include-state` says nothing under `--json`, where the stored record carries its state either way.

A log file that does not exist yet is an error, as it is today: `logs` reads a file the daemon has already written to, and lnav says which file it could not open.

Nothing about the file changes. The daemon writes what it writes, `jq` reads it the same way, and `mercury logs --json | jq` is untouched.

## What is lost

The Rust renderer's three cosmetic touches go, because lnav's line format has no equivalent:

- The timestamp is lnav's, `2026-07-21T09:14:02.114-0400`, rather than `07-21 09:14:02.114`. It is longer, and it is in local time rather than UTC.
- `duration_us=1740` rather than `took=1.74ms`. lnav renders an integer field as the integer it is.
- The duration is not coloured by how long it was. `;SELECT ... WHERE duration_us > 1000` and `:spectrogram duration_us` answer the question the colour was answering.

`mercury logs > file` no longer follows: with stdout not a terminal, `logs` runs `lnav -n`, which renders the whole file and exits. A pipeline that wants to follow uses `--json`.

## The assets

### `crates/freddie_cli/lnav/format.json`

The record envelope is `freddie_cli::logging`'s, so the format is the crate's own. `hide-extra` is `false`, so a field no call site had when this was written still appears, as an indented line under the record rather than being dropped. The fields named in `line-format` are the ones that recur across freddie's own crates; each carries a `prefix` and an empty `default-value`, and lnav drops the prefix when the value is empty, so a record renders on one line with exactly the fields it has.

```json
{
  "$schema": "https://lnav.org/schemas/format-v1.schema.json",
  "freddie_log": {
    "title": "freddie daemon log",
    "description": "One JSON object per record, as freddie_cli's tracing layer writes it.",
    "json": true,
    "hide-extra": false,
    "timestamp-field": "timestamp",
    "level-field": "level",
    "level": {
      "error": "ERROR",
      "warning": "WARN",
      "info": "INFO",
      "debug": "DEBUG",
      "trace": "TRACE"
    },
    "line-format": [
      { "field": "__timestamp__" },
      { "field": "pid", "prefix": " pid=" },
      { "field": "target", "prefix": " ", "suffix": " " },
      { "field": "__level__" },
      { "field": "message", "prefix": " " },
      { "field": "key", "prefix": " key=", "default-value": "" },
      { "field": "press", "prefix": " press=", "default-value": "" },
      { "field": "kind", "prefix": " kind=", "default-value": "" },
      { "field": "input", "prefix": " input=", "default-value": "" },
      { "field": "event", "prefix": " event=", "default-value": "" },
      { "field": "effects", "prefix": " effects=", "default-value": "" },
      { "field": "duration_us", "prefix": " duration_us=", "default-value": "" },
      { "field": "app", "prefix": " app=", "default-value": "" },
      { "field": "url", "prefix": " url=", "default-value": "" },
      { "field": "daemon", "prefix": " daemon=", "default-value": "" },
      { "field": "raw_flags", "prefix": " raw_flags=", "default-value": "" },
      { "field": "intrinsic", "prefix": " intrinsic=", "default-value": "" },
      { "field": "source_pid", "prefix": " source_pid=", "default-value": "" }
    ],
    "value": {
      "pid": { "kind": "integer", "identifier": true },
      "target": { "kind": "string", "identifier": true },
      "message": { "kind": "string" },
      "key": { "kind": "string", "identifier": true },
      "press": { "kind": "string", "identifier": true },
      "kind": { "kind": "string", "identifier": true },
      "input": { "kind": "string" },
      "event": { "kind": "string" },
      "effects": { "kind": "string" },
      "duration_us": { "kind": "integer" },
      "app": { "kind": "string", "identifier": true },
      "url": { "kind": "string", "identifier": true },
      "daemon": { "kind": "integer", "identifier": true },
      "raw_flags": { "kind": "string" },
      "intrinsic": { "kind": "string" },
      "source_pid": { "kind": "integer" },
      "state": { "kind": "string", "hidden": true }
    },
    "sample": [
      {
        "line": "{\"pid\":63568,\"timestamp\":\"2026-07-27T11:46:55.879406Z\",\"level\":\"INFO\",\"message\":\"dispatch\",\"event\":\"Key(DeviceKeyed { key: KeyEvent { key: KeyN, press: Up }, device: Laptop })\",\"effects\":\"[Emit(KeyEvent { key: KeyN, press: Up })]\",\"duration_us\":84,\"state\":\"Figaro { .. }\",\"target\":\"figaro::daemon\"}"
      }
    ]
  }
}
```

What that renders, against records out of a live figaro log:

```
2026-07-27T12:49:05.273762+0100 pid=63568 freddie_keyboard::sys::macos debug tap input=KeyEvent { key: AltLeft, press: Up } source_pid=0
2026-07-27T12:49:05.273974+0100 pid=63568 figaro::daemon info dispatch event=Key(DeviceKeyed { key: KeyEvent { key: AltLeft, press: Up }, device: Laptop }) effects=[Emit(KeyEvent { key: AltLeft, press: Up })] duration_us=47
2026-07-27T12:49:05.274166+0100 pid=63568 freddie_keyboard::sys::macos debug post key=AltLeft press=Up kind=FlagsChanged raw_flags=0x00000000 intrinsic=0x00000000
2026-07-27T12:49:05.274245+0100 pid=63568 figaro::daemon debug emitted key=AltLeft press=Up
```

A field the format does not name renders as its own indented line, so nothing is ever lost:

```
2026-07-27T12:49:05.273762+0100 pid=63568 freddie_keyboard::sys::macos debug tap
  some_new_field: whatever the call site passed
```

### `crates/freddie_cli/lnav/config.freddie.json`

The bindings go into lnav's own `default` keymap rather than a keymap of ours, so a key this file does not name keeps the binding lnav ships. The command each key runs is what lnav's own keymap runs for the equivalent default, with one correction: lnav's stock `=` runs `UPDATE lnav_views SET paused = 1 - paused` across every view, and the second press of it fails with `Expecting an integer for column number 12`. `WHERE name = 'log'` is the log view alone, and toggles both ways.

Keys are lnav's hex encoding of the UTF-8 bytes: `x70` is `p`, `x66` is `f`, `x46` is `F`.

```json
{
  "$schema": "https://lnav.org/schemas/config-v1.schema.json",
  "ui": {
    "keymap-defs": {
      "default": {
        "x70": {
          "command": ";UPDATE lnav_views SET paused = 1 - paused WHERE name = 'log'",
          "alt-msg": "p resumes"
        },
        "x66": {
          "command": ":prompt command : 'filter-in '",
          "alt-msg": "a regex; only matching records stay"
        },
        "x46": {
          "command": ":prompt command : 'filter-out '",
          "alt-msg": "a regex; matching records go"
        }
      }
    }
  }
}
```

`p` was lnav's row-details toggle, which this takes. `x`, the hidden-fields toggle, is untouched and is what `--include-state` reaches for once lnav is up.

## Change 1: ship the assets and write them out

The two files land under `crates/freddie_cli/lnav/`, are embedded with `include_str!`, and are written into a directory of the daemon's own on every `logs`. Written every time rather than once: a rebuilt binary carrying a new format has to win over the copy an older one left, and an idempotent write is how that happens without a version to compare.

The layout is the one lnav's `-I` scans. A format lives at `<dir>/formats/<name>/format.json`, and a config beside it at `<dir>/formats/<name>/config.<name>.json`.

`instance.rs`, after `log_file`:

```rust
    /// The directory `logs` writes lnav's format and keymap into, and hands to lnav with `-I`.
    ///
    /// Under the log directory because it belongs to the same daemon the log does, and because
    /// lnav reads it only when it is named on a command line: nothing else scans it, and a file
    /// left in it by an older binary is overwritten by the newer one before lnav runs.
    #[must_use]
    pub fn lnav_dir(&self) -> PathBuf {
        self.log_dir.join("lnav")
    }
```

`client.rs`, beside the other `logs` machinery:

```rust
/// lnav's format for the record envelope `logging` writes.
const LNAV_FORMAT: &str = include_str!("../lnav/format.json");

/// The keys `logs` binds on top of lnav's own keymap.
const LNAV_CONFIG: &str = include_str!("../lnav/config.freddie.json");

/// What lnav calls the format, which is what `:show-fields` and a SQL query name it by.
const LNAV_FORMAT_NAME: &str = "freddie_log";

/// Write lnav's format and keymap where lnav will read them, and answer with the directory to
/// hand it. Rewritten on every call, so a newer binary's format replaces an older one's.
fn write_lnav_assets(instance: &Instance) -> io::Result<PathBuf> {
    let dir = instance.lnav_dir();
    let format_dir = dir.join("formats").join(LNAV_FORMAT_NAME);
    std::fs::create_dir_all(&format_dir)?;
    std::fs::write(format_dir.join("format.json"), LNAV_FORMAT)?;
    std::fs::write(
        format_dir.join(format!("config.{LNAV_FORMAT_NAME}.json")),
        LNAV_CONFIG,
    )?;
    debug!("wrote lnav assets to {}", dir.display());
    Ok(dir)
}
```

Shipped on its own, this changes nothing a person sees; `lnav -I ~/Library/Logs/mercury/lnav ~/Library/Logs/mercury/mercury.log` renders the log the moment `logs` has run once.

## Change 2: `logs` runs lnav

`LogsView`'s two `bool`s become the enums they were standing in for. `include_state` is which of two things the state field does, and `json` is which of two things a record is on the way out, so each is named.

Before, in `client.rs`:

```rust
/// What `logs` renders, and how much of each record.
#[derive(Clone, Copy)]
pub(crate) struct LogsView {
    /// The least severe records to show.
    pub(crate) least: Level,
    /// Put the state field back on dispatch records.
    pub(crate) include_state: bool,
    /// Emit each record as the raw JSON it is stored as, for `jq`.
    pub(crate) json: bool,
}
```

After:

```rust
/// What `logs` shows, and how much of each record.
#[derive(Clone, Copy)]
pub(crate) struct LogsView {
    /// The least severe records to show.
    pub(crate) least: Level,
    /// Whether a dispatch record's state field is on screen.
    pub(crate) state: StateField,
    /// What a record looks like on the way out.
    pub(crate) records: Records,
}

/// Whether a dispatch record's state field is on screen. It is the whole model under `Debug` and
/// most of the line, so it is hidden until something is being debugged.
#[derive(Clone, Copy)]
pub(crate) enum StateField {
    Hidden,
    Shown,
}

/// What a record looks like on the way out.
#[derive(Clone, Copy)]
pub(crate) enum Records {
    /// Rendered by lnav, which owns the terminal for as long as it runs.
    Viewed,
    /// The JSON line as it is stored, followed as the daemon appends, for `jq`.
    Stored,
}
```

`lib.rs`, in `run_verb_on`, before:

```rust
        Verb::Logs(args) => client::logs(
            instance,
            client::LogsView {
                least: args.level,
                include_state: args.include_state,
                json: args.json,
            },
        ),
```

After:

```rust
        Verb::Logs(args) => client::logs(
            instance,
            client::LogsView {
                least: args.level,
                state: if args.include_state {
                    client::StateField::Shown
                } else {
                    client::StateField::Hidden
                },
                records: if args.json {
                    client::Records::Stored
                } else {
                    client::Records::Viewed
                },
            },
        ),
```

`StateField` and `Records` are `pub(crate)` beside `LogsView`, which is already how `lib.rs` names it.

`logs` itself, before:

```rust
/// Follow the log file: show the tail of what is there, then whatever arrives.
///
/// `tail -F` rather than a follower of our own. It waits for a file that does not exist yet, which
/// is the first run on a machine before anything has been logged, and it reopens by name if the
/// file is replaced.
///
/// Its stdout is piped rather than inherited, so each line can be dropped or shown. Its stderr and
/// its process group are inherited, so Ctrl-C reaches it and ends the follow. That is the whole
/// reason `refactors/past/mercury-start.md` puts the daemon in a group of its own.
///
/// Lines are written straight to stdout rather than traced: they are already records, out of the
/// file this is following, and tracing them would put them back into it.
pub(crate) fn logs(instance: &Instance, view: LogsView) -> ExitCode {
    let path = instance.log_file();
    info!("{}: following {}", instance.display_name(), path.display());

    // Asked once: a pipeline gets the file's plain text, a terminal gets colour.
    let color = std::io::stdout().is_terminal();
    let mut out = std::io::stdout().lock();

    match follow(&path, &view, color, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            warn!(
                "{}: could not read {}: {e}",
                instance.display_name(),
                path.display()
            );
            ExitCode::FAILURE
        }
    }
}
```

After. The comment claimed `tail -F`, which this stopped shelling out to some time ago, and the piped-stdout paragraph described a subprocess that is not there either. What replaces both is what the function now decides:

```rust
/// Open the log file: in lnav, which follows it and gives it keys, or as the records it stores.
///
/// Nothing here renders a record. lnav does, out of the format written beside the log, so the
/// terminal's rendering and a `--json` consumer's parsing come from the one file the daemon wrote.
///
/// `--json` is written straight to stdout rather than traced: those lines are already records, out
/// of the file this is following, and tracing them would put them back into it.
pub(crate) fn logs(instance: &Instance, view: LogsView) -> ExitCode {
    let path = instance.log_file();
    info!("{}: following {}", instance.display_name(), path.display());

    match view.records {
        Records::Viewed => view_in_lnav(instance, &path, view),
        Records::Stored => stream(instance, &path, view.least),
    }
}

/// Hand the terminal to lnav, with the format and keymap it needs and the flags this invocation
/// asked for.
///
/// Replaces this process rather than spawning one: lnav owns the terminal, its exit status is the
/// verb's, and there is nothing left for this process to do once lnav is up. `-n` when stdout is
/// not a terminal, which renders the file and exits rather than driving a screen that is a pipe.
fn view_in_lnav(instance: &Instance, path: &Path, view: LogsView) -> ExitCode {
    let assets = match write_lnav_assets(instance) {
        Ok(dir) => dir,
        Err(e) => {
            warn!("{}: could not write lnav's format: {e}", instance.display_name());
            return ExitCode::FAILURE;
        }
    };

    let mut command = Command::new("lnav");
    command.arg("-I").arg(&assets);
    if !std::io::stdout().is_terminal() {
        command.arg("-n");
    }
    command.arg("-c").arg(format!(
        ":set-min-log-level {}",
        view.least.as_str().to_lowercase()
    ));
    if let StateField::Shown = view.state {
        command
            .arg("-c")
            .arg(format!(":show-fields {LNAV_FORMAT_NAME}.state"));
    }
    command.arg(path);

    match become_lnav(&mut command) {
        Ok(code) => code,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!(
                "{}: lnav is not on PATH, and `logs` reads the log with it: brew install lnav",
                instance.display_name()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            warn!("{}: could not run lnav: {e}", instance.display_name());
            ExitCode::FAILURE
        }
    }
}

/// Replace this process with `command`. On unix that is `exec`, which returns only on failure. On
/// windows there is no `exec`, so the process stays and waits.
#[cfg(unix)]
fn become_lnav(command: &mut Command) -> io::Result<ExitCode> {
    use std::os::unix::process::CommandExt as _;
    Err(command.exec())
}

#[cfg(windows)]
fn become_lnav(command: &mut Command) -> io::Result<ExitCode> {
    let status = command.status()?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
```

`follow` is renamed `stream` and takes the level rather than the whole view, since the rendering it was branching on is gone:

```rust
/// Write `path`'s last [`BACKLOG_LINES`] records and then whatever is appended, as they are
/// stored, filtered to `least` and above.
fn stream(instance: &Instance, path: &Path, least: Level) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match follow(path, least, &mut out) {
        // The follow ends only when stdout closes, which is the pipeline this was feeding going
        // away: `logs --json | head -3` got its lines and left. A clean finish, not a failure.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            warn!(
                "{}: could not read {}: {e}",
                instance.display_name(),
                path.display()
            );
            ExitCode::FAILURE
        }
    }
}
```

`follow`'s signature loses `view` and `color`:

```rust
fn follow(path: &Path, least: Level, out: &mut impl Write) -> io::Result<()> {
```

Its body is unchanged but for the two `show_record` calls, which now read `show_record(out, record, least)`.

## Change 3: delete the renderer

With lnav doing the rendering, everything in `client.rs` that formatted a record goes: `show`, `as_text`, `format_timestamp`, `format_duration`, `duration_color`, `level_color`, `DIM`, `RESET`, and `VERBOSE_FIELDS`. `Cow` leaves the imports with `as_text`.

`Record` keeps only what the level filter reads:

```rust
/// One record out of the log file, as far as the level filter is concerned.
///
/// Only the level, because nothing here renders a record any more: lnav does, and `--json` writes
/// the line as it stands. A line that does not parse as this is not a record.
#[derive(serde::Deserialize)]
struct Record {
    level: String,
}
```

`show_record` becomes the filter it now is:

```rust
/// Write one record to `out` if `least` admits it. `Break` when `out` has closed.
///
/// A line that is not a record is written as it stands: a file written by an older daemon, or
/// something that reached the file without going through the formatter. Hiding what cannot be
/// classified is how a log loses the one line that mattered, so a record whose level does not
/// parse is written on the same reasoning.
fn show_record(out: &mut impl Write, line: &str, least: Level) -> ControlFlow<()> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let Ok(record) = serde_json::from_str::<Record>(line) else {
        return broke(writeln!(out, "{line}").is_err());
    };
    if record
        .level
        .parse::<Level>()
        .is_ok_and(|level| level > least)
    {
        return ControlFlow::Continue(());
    }
    broke(writeln!(out, "{line}").is_err())
}
```

The tests in `client.rs` that assert the rendering go with it: `the_state_is_left_out_unless_asked_for`, `a_record_reads_as_its_parts`, `a_timestamp_reads_as_the_month_day_and_wall_clock_time`, `a_duration_reads_pretty_as_took`, and `a_slow_duration_is_coloured`. `a_line_that_is_not_a_record_does_not_parse` stays, and two take the place of the five that went:

```rust
#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;

    use tracing::Level;

    use super::{LNAV_CONFIG, LNAV_FORMAT, Record, show_record};

    const DISPATCH: &str = r#"{"pid":1,"timestamp":"2026-07-21T09:14:02.114Z","level":"INFO","message":"dispatch","event":"Key(KeyR)","effects":"[]","state":"Mercury { .. }","target":"mercury::daemon"}"#;

    fn streamed(line: &str, least: Level) -> (String, ControlFlow<()>) {
        let mut out = Vec::new();
        let flow = show_record(&mut out, line, least);
        (String::from_utf8(out).expect("the record is utf8"), flow)
    }

    // A record goes out as the line it came in as: `--json` is the file's own text, and what
    // renders it is lnav.
    #[test]
    fn a_record_is_written_as_it_is_stored() {
        let (out, _) = streamed(DISPATCH, Level::DEBUG);
        assert_eq!(out.trim_end(), DISPATCH);
    }

    #[test]
    fn a_record_below_the_level_is_not_written() {
        let (out, _) = streamed(DISPATCH, Level::WARN);
        assert!(out.is_empty(), "{out}");
    }

    #[test]
    fn a_line_that_is_not_a_record_does_not_parse() {
        assert!(
            serde_json::from_str::<Record>("Boot-out failed: 36: Operation now in progress")
                .is_err()
        );
    }

    // The two assets are what lnav parses, so a comma out of place is a broken `logs`, and the
    // build is where that should be caught rather than the first run after it.
    #[test]
    fn the_lnav_assets_are_json() {
        serde_json::from_str::<serde_json::Value>(LNAV_FORMAT).expect("the format is json");
        serde_json::from_str::<serde_json::Value>(LNAV_CONFIG).expect("the config is json");
    }
}
```

## Change 4: the docs that describe `logs`

`CLAUDE.md`, in "Logs", before:

```
`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane with `mercury logs`, which follows the file and shows records at `info` and above; `mercury logs --level debug` widens that.
```

After:

```
`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane with `mercury logs`, which opens the file in lnav, follows it, and shows records at `info` and above; `mercury logs --level debug` widens that, and `:set-min-log-level debug` widens it without leaving lnav.
```

And, two paragraphs on, before:

```
`mercury logs` leaves the state out. It is the whole model under `Debug`, which is most of a dispatch record and is wanted while something is being debugged; `mercury logs --include-state` puts it back, and `mercury logs --json` gives the records as stored.
```

After:

```
`mercury logs` opens the log in lnav (`brew install lnav`), following, with a format that renders one record per line. `p` pauses, `f` and `F` filter records in and out by regex, `/` searches, `e` and `w` jump to the next error and warning, `x` shows the state field, and `;` is SQL over the records. The state is left out until asked for: it is the whole model under `Debug`, which is most of a dispatch record. `mercury logs --include-state` starts with it shown, and `mercury logs --json` skips lnav and gives the records as stored, following as they arrive.
```

`CLAUDE.md`, in "Nothing is printed", before:

```
Three things stay unrouted, because none of them is mercury's own output. clap writes `--help`, `--version`, and parse errors itself and exits. `tail`, under `mercury logs`, writes the file's own contents, which tracing would append back into the file being followed. Tests print for whoever is reading the test run.
```

After:

```
Three things stay unrouted, because none of them is mercury's own output. clap writes `--help`, `--version`, and parse errors itself and exits. lnav, under `mercury logs`, writes the file's own contents, which tracing would append back into the file being followed. Tests print for whoever is reading the test run.
```

`docs-website/docs/getting-started-with-mercury.md`, in "Watching what it does", before:

```markdown
Run `mercury logs` alongside it. Every dispatched event writes one record carrying the event, the effects it produced, and the resulting state.
```

After:

```markdown
Run `mercury logs` alongside it. Every dispatched event writes one record carrying the event, the effects it produced, and the resulting state.

It opens the log in [lnav](https://lnav.org), so it needs one installed (`brew install lnav`). lnav follows the file, and the keys are its own: `p` pauses the scroll, `f` and `F` filter records in and out by regular expression, `/` searches and `n` walks the hits, `e` and `w` jump to the next error and warning, `x` shows the model state a record carries, and `q` quits. `mercury logs --json` skips lnav and writes the records as they are stored, for `jq`.
```

The boot-state snippet below it is still what a record carries, so it stays.

`crates/freddie_cli/src/lib.rs`'s crate doc says the verbs "tail a log". They open one:

```rust
//! is the only process that holds it. Nothing here looks inside the process: the verbs read a lock
//! file, spawn a binary, signal a pid, and open a log in lnav, and every one of those works the
//! same whatever the daemon is for.
```

`client.rs`'s `logs` doc comment claims `tail -F`, which the follower it describes stopped being some time ago. It goes with the function it sits on.
