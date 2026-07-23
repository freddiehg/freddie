# the log file is records, not lines

`mercury logs` decides what to show by looking at text: the level is the third whitespace-separated token, and the colour is applied by splitting the line on the level's own name. It can filter a record out and it can colour one, and that is the whole of what it can do, because a formatted line has no parts it can name.

What it cannot do is show a record without one of its fields. The dispatch record carries the event, the effects, and the whole state, and the state is most of the line and the least of what is read. Cutting it out means finding `state=` in rendered `Debug` output and hoping nothing inside it looks like the thing being searched for.

So the file becomes one JSON object per line, `mercury logs` reads fields rather than tokens, and the state is a field it leaves out unless asked.

## What the file looks like

```json
{"pid":48213,"timestamp":"2026-07-21T09:14:02.114Z","level":"INFO","target":"mercury::daemon","fields":{"message":"dispatch","event":"Key(KeyEvent { key: KeyR, press: Down, flags: (empty) })","effects":"[Timer(TimerEffect { delay: 3s })]","state":"Mercury { foreground: ..., windows: Windows, ... }"}}
```

The fields are strings, because they are what `?value` rendered. What changes is that each one is addressable: `state` is a value under a key rather than a run of characters in a sentence.

## Change 1: the file layer writes JSON

`crates/mercury/Cargo.toml`, before:

```toml
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

after:

```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

`crates/mercury/src/logging.rs`, before:

```rust
    let file = fmt::layer()
        .with_writer(WithPid(tracing_appender::rolling::never(&dir, LOG_FILE)))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);
```

after:

```rust
    let file = fmt::layer()
        .json()
        // Flat: one object per record, with its fields under `fields` and nothing else nested.
        // `mercury logs` renders a record from those keys, and a span list would be keys it has
        // no line to put.
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(WithPid(tracing_appender::rolling::never(&dir, LOG_FILE)))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);
```

The daemon's own terminal layer is untouched. It is read as it is written, by a person watching a run, so it stays the human format; the file is the one with a reader that parses it.

## Change 2: the pid is a field

`WithPid` prefixes `pid=48213 ` to the formatted record, which in front of a JSON object produces a line that is neither. It becomes the object's first key instead, which is the same splice at the same place.

`crates/mercury/src/logging.rs`, before:

```rust
/// This process's stamp, built once. A pid does not change under a running process.
fn stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| format!("pid={} ", std::process::id()))
}
```

after:

```rust
/// This process's stamp: the opening brace of the record's object and the pid inside it, so
/// splicing it in front of a record whose own opening brace has been taken off puts the pid first.
///
/// Built once. A pid does not change under a running process.
fn stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| format!("{{\"pid\":{},", std::process::id()))
}
```

and the write, before:

```rust
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        STAMPED.with_borrow_mut(|line| {
            line.clear();
            line.extend_from_slice(stamp().as_bytes());
            line.extend_from_slice(record);
            self.0.write_all(line)
        })?;
        Ok(record.len())
    }
```

after:

```rust
    /// One `write_all` for the stamp and the record together.
    ///
    /// Two calls would be two appends, and another process may append between them, which would
    /// leave a stamp attached to a stranger's record. Building the line first is what keeps a
    /// record whole against the other writers this exists for.
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
```

## Change 3: the logs verb reads records

`crates/mercury/src/client.rs`. `record_level` and the split-on-the-level colouring both go; what replaces them is one deserialize.

```rust
/// One record out of the log file.
///
/// `fields` is a map rather than a struct, because its keys are whatever the call site passed:
/// `message` is always there and the rest are the record's own.
#[derive(serde::Deserialize)]
struct Record {
    pid: u32,
    timestamp: String,
    level: String,
    target: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

/// The fields `mercury logs` leaves out unless asked for them.
///
/// The state is the whole model rendered with `Debug`, which is most of a dispatch record and is
/// read when something is being debugged and not before.
const VERBOSE_FIELDS: &[&str] = &["state"];
```

and the rendering:

```rust
/// Show one record, colouring it the way the daemon's own terminal would have.
///
/// The file is written with ANSI off, because a person or an agent reads it with `grep` and `jq`,
/// and escapes in the file would defeat both. Colour is added here instead.
fn show(out: &mut impl Write, record: &Record, args: &LogsArgs, color: bool) -> io::Result<()> {
    let (dim, reset, level_color) = if color {
        (DIM, RESET, level_color(record.level.parse().unwrap_or(Level::INFO)))
    } else {
        ("", "", "")
    };
    write!(
        out,
        "{dim}{} pid={} {}{reset} {level_color}{}{reset}",
        record.timestamp, record.pid, record.target, record.level
    )?;
    if let Some(message) = record.fields.get("message") {
        write!(out, " {}", as_text(message))?;
    }
    for (key, value) in &record.fields {
        if key == "message" || (!args.include_state && VERBOSE_FIELDS.contains(&key.as_str())) {
            continue;
        }
        write!(out, " {dim}{key}={reset}{}", as_text(value))?;
    }
    writeln!(out)
}

/// A field as it reads: a string without its quotes, anything else as JSON.
fn as_text(value: &serde_json::Value) -> Cow<'_, str> {
    match value {
        serde_json::Value::String(s) => Cow::Borrowed(s),
        other => Cow::Owned(other.to_string()),
    }
}
```

The follow loop, before:

```rust
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let level = record_level(&line);
        if level.is_none_or(|level| level <= args.level) {
            if show(&mut out, &line, level, color).is_err() {
                break;
            }
        }
    }
```

after:

```rust
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        // A line that is not a record is shown as it stands: a file written by an older mercury,
        // or something that reached it without going through the formatter. Hiding what we cannot
        // classify is how a log loses the one line that mattered.
        let Ok(record) = serde_json::from_str::<Record>(&line) else {
            if writeln!(out, "{line}").is_err() {
                break;
            }
            continue;
        };
        if record.level.parse().is_ok_and(|level: Level| level > args.level) {
            continue;
        }
        // A closed stdout is the pipeline this was feeding going away, which ends the follow
        // rather than being worth a word about.
        if args.json {
            if writeln!(out, "{line}").is_err() {
                break;
            }
        } else if show(&mut out, &record, args, color).is_err() {
            break;
        }
    }
```

A record whose level does not parse is shown, on the same reasoning as a line that is not a record.

## Change 4: the two flags

`crates/mercury/src/cli.rs`, before:

```rust
pub struct LogsArgs {
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    #[arg(long, default_value = DEFAULT_LOG_LEVEL)]
    pub level: Level,
}
```

after:

```rust
pub struct LogsArgs {
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    ///
    /// The file always records `debug`, whatever this says, so this widens or narrows what reaches
    /// the terminal and never what is kept. Defaults to what a daemon's own terminal defaults to.
    #[arg(long, default_value = DEFAULT_LOG_LEVEL)]
    pub level: Level,

    /// Include the model state on each dispatch record.
    ///
    /// It is the whole model under `Debug` and it is most of the line, so it is left out of a
    /// follow that is watching what happened rather than reading what the model became. Off unless
    /// asked for.
    #[arg(long)]
    pub include_state: bool,

    /// Write each record as the JSON it is stored as, for `jq`.
    #[arg(long)]
    pub json: bool,
}
```

`--level` and `--include-state` are separate axes on purpose: one chooses which records, the other chooses how much of one. `--include-state --level debug` is a full read of the file, which is what it was before this.

`--json` ignores `--include-state`, because the raw record is the raw record.

## Change 5: what `CLAUDE.md` says about the log

The Logs section describes a text file read with `cat` and `grep`, and a dispatch record whose state is on the line. Both change.

Before:

```
It holds one record per dispatched event, carrying the event, the effects it produced, and the
resulting state on a single line, plus each key emitted, each app foregrounded, and the raw
frontmost-app changes `freddie_app_nav` observed.
```

after:

```
Every line is one JSON object: `pid`, `timestamp`, `level`, `target`, and the record's own fields
under `fields`. So `jq` reads it, and `mercury logs` renders it rather than parsing text.

It holds one record per dispatched event, carrying the event, the effects it produced, and the
resulting state, plus each key emitted, each app foregrounded, and the raw frontmost-app changes
`freddie_app_nav` observed.

`mercury logs` leaves the state out. It is the whole model under `Debug`, which is most of a
dispatch record and is wanted while something is being debugged; `mercury logs --include-state`
puts it back, and `mercury logs --json` gives the records as stored.
```

and the pid paragraph, before:

```
Every record carries the pid of the process that wrote it, because a client verb and the daemon
both append to the one file. `pid=` is always the writer; a field naming some other process says
which, as `stop`'s `daemon=` does.
```

after:

```
Every record carries the pid of the process that wrote it, because a client verb and the daemon
both append to the one file. `pid` is always the writer; a field naming some other process says
which, as `stop`'s `daemon=` does.
```

## Tests

`crates/mercury/src/client.rs`'s tests are built on `record_level` and a rendered line. They become records.

```rust
fn record(line: &str) -> Record {
    serde_json::from_str(line).expect("a record")
}

fn shown(line: &str, args: &LogsArgs, color: bool) -> String {
    let mut out = Vec::new();
    show(&mut out, &record(line), args, color).expect("writing to a Vec");
    String::from_utf8(out).expect("the record is utf8")
}

const DISPATCH: &str = r#"{"pid":1,"timestamp":"2026-07-21T09:14:02.114Z","level":"INFO","target":"mercury::daemon","fields":{"message":"dispatch","event":"Key(KeyR)","state":"Mercury { .. }"}}"#;

#[test]
fn the_state_is_left_out_unless_asked_for() {
    let shown_without = shown(DISPATCH, &LogsArgs { include_state: false, ..logs_args() }, false);
    assert!(shown_without.contains("event=Key(KeyR)"));
    assert!(!shown_without.contains("state="));

    let shown_with = shown(DISPATCH, &LogsArgs { include_state: true, ..logs_args() }, false);
    assert!(shown_with.contains("state=Mercury { .. }"));
}

#[test]
fn a_record_reads_as_its_parts() {
    let line = shown(DISPATCH, &logs_args(), false);
    assert_eq!(
        line.trim_end(),
        "2026-07-21T09:14:02.114Z pid=1 mercury::daemon INFO dispatch event=Key(KeyR)"
    );
}

#[test]
fn a_line_that_is_not_a_record_is_not_a_record() {
    assert!(serde_json::from_str::<Record>("Boot-out failed: 36: Operation now in progress").is_err());
}
```

and one that the writer produces a line the reader can read, which is the seam this whole change rests on:

```rust
#[test]
fn what_the_writer_writes_is_a_record() {
    // The stamp splices in front of a record whose opening brace has been taken off.
    let formatted = br#"{"timestamp":"t","level":"INFO","target":"x","fields":{"message":"m"}}"#;
    let mut written = Vec::new();
    PidStamped(&mut written).write_all(formatted).expect("writing to a Vec");
    let record: Record = serde_json::from_slice(&written).expect("a record");
    assert_eq!(record.pid, std::process::id());
    assert_eq!(record.level, "INFO");
}
```
