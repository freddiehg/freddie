# daemon flags

A daemon's settings live in three places today, and only one of them is a mechanism. `--port` is a clap flag on the hidden `daemon` verb, mirrored onto `start` and `restart` and forwarded to the spawned child. `LOG_LEVEL` is an environment variable read once inside `logging::init`. Everything else, the keymap included, is compiled in. Nothing reads a config file, nothing is re-readable while a daemon runs, and the launchd job gets neither the flag nor a shell's variables.

So this doc states where a daemon setting goes, what a restart does with it, and how an installed job carries it. Then it applies the answer to the first setting that needs a new home: what the log file records, which is a constant today and becomes opt-in.

## the rule

A daemon setting is a flag on the `daemon` verb. It is mirrored by `start` and `restart`, forwarded by `spawn_daemon` to the child they put somewhere, written into `ProgramArguments` by `install`, and read once, at construction. Changing it is `restart` with the new flag.

Three things follow, and each is a question this doc is answering rather than a preference.

A setting is not a config file. Two sources of truth for one value need a rule about which wins, a parse, a schema, and an answer to whether a running daemon re-reads the file. The last one is the expensive part: re-reading means a watcher and a staleness question for a value the daemon consumed at construction, and not re-reading is what a flag already is with none of the parsing. `--help` also stops being the list of what can be set. The plist already is the persistent configuration for an installed daemon, and `install` writes it rather than a person editing it.

A setting is not primarily an environment variable, because clap's `env` attribute makes every flag one for free. `--port` already spells this:

```rust
// crates/mercury/src/main.rs
#[derive(clap::Args, Debug)]
pub struct MercuryArgs {
    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}
```

One declaration, one name in `--help`, and both spellings resolve to the same field.

`LOG_LEVEL` stays a variable and does not become a flag. The reason is already written where it is declared, and it is a reason about terminals rather than about variables:

```rust
// crates/freddie_cli/src/logging.rs
/// The environment variable a daemon reads its terminal filter from.
///
/// Not a flag: the only invocation with a terminal to filter is one a person typed in
/// front of, and `daemon` is hidden, spawned by `start` with its output at /dev/null, and
/// run by launchd with no terminal at all. A variable serves the one case a flag would.
pub const LOG_LEVEL: &str = "LOG_LEVEL";
```

The file is the opposite case. It exists for the detached daemon and the launchd job, which is exactly where a flag reaches and a variable typed in front of a shell does not. So the setting this doc adds is a flag, and that comment is what tells the two apart.

## what a restart does

`restart` spawns the replacement from the argv this invocation typed, and inherits nothing from the daemon it replaced. `TypedArgs::argv` re-emits only values whose `ValueSource` is `CommandLine`, so:

- `mercury restart --log-file-level debug` runs the replacement with that filter.
- `mercury restart` alone runs it with every flag at its default.

Inheriting the running daemon's flags is the alternative, and it is not what happens. The client would have to read the daemon's argv out of the process table, and a daemon's configuration would then be a function of the history of restarts rather than of the command that started it. A daemon's flags are a function of one command line, which is the property the model already has with respect to its events.

The cost is worth stating because it is the thing a person trips on: turning a setting on for a debugging session and back off afterwards is two restarts, and the second one is the bare `restart`.

## what install carries

`install` takes no flags today, and the job it writes takes none either:

```rust
// crates/mercury/src/agent.rs
    fn running(program: &Path) -> Self {
        Self {
            label: label(),
            program_arguments: vec![
                program.to_string_lossy().into_owned(),
                DAEMON_VERB.to_owned(),
            ],
            // …
        }
    }
```

So an installed mercury resolves every setting from its default, or from whatever launchd's environment happens to hold, and a flag typed at `install` time is discarded. `install` takes the same flags the daemon does, and writes them into `ProgramArguments`.

There is one difference from `spawn_daemon`, and it is not cosmetic. A child spawned by `start` inherits this process's environment and resolves the same values from it, which is why an env-sourced value is left out of the re-emission. A launchd job inherits nothing, so a value this invocation resolved from a variable has to be written down or it is lost. `LOG_FILE_LEVEL=debug mercury install` must not quietly install a job at the default.

`crates/freddie_cli/src/lib.rs`, before:

```rust
impl TypedArgs<'_> {
    /// Re-emit the app's flags as argv for the daemon this process is about to spawn.
    pub(crate) fn argv<TApp: App>(&self) -> Vec<String> {
        // …one `push_typed` per arg set…
    }

    /// Re-emit one arg set's typed flags onto `argv`.
    fn push_typed(&self, argv: &mut Vec<String>, ids: &[clap::Id]) {
        for id in ids {
            if self.matches.value_source(id.as_str()) == Some(ValueSource::CommandLine) {
                // …
            }
        }
    }
}
```

After:

```rust
/// Which of the values this invocation resolved a re-emission has to spell out.
///
/// The two consumers differ in what the process on the other end starts with, so this is a
/// property of that process rather than a preference at the call site.
#[derive(Clone, Copy, Debug)]
pub enum Inherits {
    /// The child inherits this process's environment and resolves the same values from it, so a
    /// value that came from a variable is left out and a default is left out with it. `start` and
    /// `restart` spawn one of these.
    Environment,
    /// The process starts with none of this environment, so every value this invocation resolved
    /// has to be written down. launchd's job is one of these: `install` runs in a shell and the
    /// job does not.
    Nothing,
}

impl Inherits {
    /// Whether a value from `source` has to be re-emitted.
    const fn must_spell_out(self, source: Option<ValueSource>) -> bool {
        match self {
            Self::Environment => matches!(source, Some(ValueSource::CommandLine)),
            Self::Nothing => matches!(
                source,
                Some(ValueSource::CommandLine | ValueSource::EnvVariable | ValueSource::DefaultValue)
            ),
        }
    }
}

impl TypedArgs<'_> {
    /// Re-emit the app's flags as argv for a process that will run the daemon.
    ///
    /// `pub` rather than `pub(crate)`: an app's own `install` verb writes the same argv into the
    /// job it registers, and that verb is the app's rather than this crate's.
    pub fn argv<TApp: App>(&self, inherits: Inherits) -> Vec<String> {
        // …one `push_typed` per arg set, each handed `inherits`…
    }

    /// Re-emit one arg set's flags onto `argv`.
    fn push_typed(&self, argv: &mut Vec<String>, ids: &[clap::Id], inherits: Inherits) {
        for id in ids {
            if inherits.must_spell_out(self.matches.value_source(id.as_str())) {
                // …
            }
        }
    }
}
```

`client::start` and `client::restart` pass `Inherits::Environment`, which is what they do today under a name.

`spawn_daemon`'s own doc closes on a sentence this change falsifies. Before:

```rust
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer then has nowhere
/// to write, which is why `--log-level` is not passed through: it governs a terminal this child
/// does not have. The log file records `debug` regardless, and `logs` reads that.
```

After:

```rust
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer then has nowhere
/// to write, which is why `LOG_LEVEL` is not spelled out: it governs a terminal this child does
/// not have, and the child inherits the variable anyway. `--log-file-level` is spelled out, because
/// the file is the sink a child with no terminal writes to.
```

mercury's `install` grows the flags and the re-emission. `crates/mercury/src/main.rs`, before:

```rust
    /// Register this binary as a login agent, so mercury starts with the session.
    Install,
```

After:

```rust
    /// Register this binary as a login agent, so mercury starts with the session.
    ///
    /// The flags typed here are the flags the job runs with, because launchd starts it with none
    /// of this shell's environment. Install again to change them.
    Install(InstallArgs),
```

```rust
/// What `install` can be told: everything the daemon it registers can be told.
#[derive(clap::Args, Debug)]
pub struct InstallArgs {
    #[command(flatten)]
    pub app: MercuryArgs,

    #[arg(long, env = freddie_cli::LOG_FILE_LEVEL, default_value = freddie_cli::DEFAULT_LOG_FILE_LEVEL)]
    pub log_file_level: String,
}
```

`crates/mercury/src/agent.rs`, before:

```rust
    fn running(program: &Path) -> Self {
        Self {
            label: label(),
            program_arguments: vec![
                program.to_string_lossy().into_owned(),
                DAEMON_VERB.to_owned(),
            ],
```

After:

```rust
    /// The agent that runs `program` with `flags`.
    ///
    /// The flags follow the verb, which is where clap wants a subcommand's arguments, and they are
    /// spelled out rather than left to the environment: launchd starts the job with none of the
    /// environment `install` was run in.
    fn running(program: &Path, flags: Vec<String>) -> Self {
        let mut program_arguments = vec![
            program.to_string_lossy().into_owned(),
            DAEMON_VERB.to_owned(),
        ];
        program_arguments.extend(flags);
        Self {
            label: label(),
            program_arguments,
```

and `install_agent` takes the argv and hands it down:

```rust
fn install_agent(flags: Vec<String>) -> Result<PathBuf, NotInstalled> {
    let program = std::env::current_exe().map_err(NotInstalled::NoExe)?;
    let path = plist_path().ok_or(NotInstalled::NoHome)?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(NotInstalled::Unwritable)?;
    }
    plist::to_file_xml(&path, &Agent::running(&program, flags))
        .map_err(NotInstalled::Unserializable)?;
```

The resulting plist, for `mercury install --port 4000`:

```xml
	<key>ProgramArguments</key>
	<array>
		<string>/Users/…/bin/mercury</string>
		<string>daemon</string>
		<string>--port</string>
		<string>4000</string>
		<string>--log-file-level</string>
		<string>warn</string>
	</array>
```

`install` is already idempotent and boots the old job out before bootstrapping the new one, so installing again is how the flags change.

## the first setting: what the file records

The file's filter is a constant, and it is the only sink with no way to ask it for less.

```rust
// crates/freddie_cli/src/logging.rs
/// What the log file records, always. Deliberately not tied to the terminal's
/// filter: the file is the record of what happened, so quieting the terminal must
/// never quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;
```

At `debug` the file takes one dispatch record per event, carrying the whole model under `Debug`, plus a `post`, a `tapped`, and an `emitted` line per keystroke. Measured on figaro: 220 MB, and 710 bytes per dispatch record. `bounded-log.md` measured mercury at 443 MB over thirteen days, about 34 MB a day, and named the other half of the problem: a record of which keys were pressed, in order, with timestamps, is what was typed, and it sits in `~/Library/Logs` readable by anything running as the user.

### the default

```rust
/// What the log file records when nothing asks for more.
///
/// `warn`, not `off`: the panic hook writes an `error!` and then aborts, and setup failures are
/// `warn!`, so this is the level at which the file still says why a daemon died or refused to
/// start. What it drops is everything routine, which is all of the volume and all of the
/// keystrokes. `--log-file-level debug` is the run that is being debugged.
pub const DEFAULT_LOG_FILE_LEVEL: &str = "warn";
```

`off` is available and is not the default, because a file that keeps nothing cannot answer the one question a log is kept for. A daemon at `warn` writes a few lines a day, and a daemon that crashed writes the reason.

### the flag

```rust
// crates/freddie_cli/src/logging.rs
/// The environment variable the log file reads its filter from, for a flag that was not typed.
///
/// Separate from [`LOG_LEVEL`], which is the terminal's alone: quieting a terminal must not quiet
/// the file, and widening the file must not fill a terminal. clap resolves this for the flag, so
/// nothing reads it directly.
pub const LOG_FILE_LEVEL: &str = "LOG_FILE_LEVEL";
```

`crates/freddie_cli/src/verb.rs`, before:

```rust
/// What the foreground daemon can be told: which daemon to be, and what the app asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<I: clap::Args, F: clap::Args> {
    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}
```

After, and the same three lines on `StartArgs` and `RestartArgs`:

```rust
/// What the foreground daemon can be told: which daemon to be, what the app asks for itself, and
/// what its log file keeps.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<I: clap::Args, F: clap::Args> {
    /// What the log file records: `off`, a level, or a `tracing_subscriber` filter such as
    /// `warn,mercury=debug`.
    ///
    /// Read once, at startup. `restart --log-file-level debug` is how a running daemon comes to
    /// keep more, and a bare `restart` puts it back.
    #[arg(long, env = crate::logging::LOG_FILE_LEVEL, default_value = crate::logging::DEFAULT_LOG_FILE_LEVEL)]
    pub log_file_level: String,

    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}
```

The three verbs that take it are the three that put a daemon somewhere, which is the same three that take `TApp::DaemonArgs`. `status`, `logs`, and `stop` do not: they find a daemon rather than configure one, and their own records go to the file at the default.

`crates/freddie_cli/src/verb.rs`, added beside `Verb::id`:

```rust
    /// What this invocation's own records are filtered by on the way to the file.
    ///
    /// The three verbs that configure a daemon say; the three that only find one take the
    /// default, since a client verb writes a handful of lines and none of them is what the flag
    /// is about.
    pub(crate) fn log_file_level(&self) -> &str {
        match self {
            Self::Start(args) => &args.log_file_level,
            Self::Restart(args) => &args.log_file_level,
            Self::Daemon(args) => &args.log_file_level,
            Self::Status(_) | Self::Logs(_) | Self::Stop(_) => {
                crate::logging::DEFAULT_LOG_FILE_LEVEL
            }
        }
    }
```

### init

Both sinks now resolve a filter from a string that may not be one, so the fallback is written once.

`crates/freddie_cli/src/logging.rs`, added:

```rust
/// The filter `directives` asks for, or `fallback` when it is not a filter.
///
/// A run with an unparseable directive still logs, and the file says what was wrong with it. The
/// complaint goes into `setup` rather than out, because there is no subscriber yet to say it to.
fn filter_from(directives: &str, fallback: &str, setup: &mut Vec<String>) -> EnvFilter {
    EnvFilter::try_new(directives).unwrap_or_else(|e| {
        setup.push(format!(
            "{directives:?} is not a log filter ({e}); using {fallback}"
        ));
        EnvFilter::new(fallback)
    })
}
```

`init`, before:

```rust
pub(crate) fn init(instance: &Instance, terminal: Terminal) {
    // …
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

After:

```rust
pub(crate) fn init(instance: &Instance, terminal: Terminal, log_file_level: &str) {
    // …
        .with_ansi(false)
        .with_filter(filter_from(
            log_file_level,
            DEFAULT_LOG_FILE_LEVEL,
            &mut setup,
        ));

    let registry = tracing_subscriber::registry().with(file);
    match terminal {
        Terminal::Daemon => {
            let directives =
                std::env::var(LOG_LEVEL).unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_owned());
            registry
                .with(
                    fmt::layer()
                        .with_writer(io::stderr)
                        .with_filter(filter_from(&directives, DEFAULT_LOG_LEVEL, &mut setup)),
                )
                .init();
        }
        Terminal::Client => registry.with(client_terminal()).init(),
    }
```

`FILE_LEVEL` and its `LevelFilter` import go. `client_terminal` still uses `LevelFilter`, so the import stays.

`crates/freddie_cli/src/lib.rs`, before:

```rust
    logging::init(&instance, verb.terminal());
```

After:

```rust
    logging::init(&instance, verb.terminal(), verb.log_file_level());
```

and the client-only call site, before:

```rust
    logging::init(instance, logging::Terminal::Client);
```

After:

```rust
    logging::init(instance, logging::Terminal::Client, logging::DEFAULT_LOG_FILE_LEVEL);
```

### what logs shows now

`mercury logs` follows the file and renders what is in it. On a default daemon the file holds warnings and nothing else, so a follow shows nothing, and that is the one place this change is confusing rather than merely quieter. The client says so, because it is the only party that can: it knows what it was asked to show and what it found.

`crates/freddie_cli/src/verb.rs`, the `--level` doc, before:

```rust
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    ///
    /// The file always records `debug`, whatever this says, so this widens or narrows what
    /// reaches the terminal and never what is kept.
```

After:

```rust
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    ///
    /// This narrows what reaches the terminal and never what is kept. What is kept is the
    /// daemon's `--log-file-level`, which defaults to `warn`, so asking here for more than the
    /// daemon was started with shows what the file has and no more.
```

`crates/freddie_cli/src/client.rs`, in `logs`, added after the backlog is shown and before the follow begins:

```rust
/// Said once when the backlog held nothing this view would show.
///
/// On a daemon started at the default that is what a quiet file looks like from here, and
/// without this line it looks like a broken follow. `warn!`, because a follow that will show
/// nothing is something the person who typed it has to see.
const NOTHING_KEPT: &str = "no records to show; the file keeps what the daemon's \
    --log-file-level asked for, and `restart --log-file-level debug` widens it";
```

```rust
    if shown == 0 {
        warn!("{NOTHING_KEPT}");
    }
```

`shown` is the count `show_record` already produces one of per rendered line, threaded out of the backlog pass.

### the workflow this makes

Debugging a run is two commands rather than one, and the first is the new part:

```
mercury restart --log-file-level debug
mercury logs --level debug
```

and afterwards:

```
mercury restart
```

What is given up: a problem that has already happened on a default daemon is not in the file beyond its warnings, and reproducing it means restarting into a wider filter and doing it again. That is the price of the file not being a typing log, and it is the reason the default is `warn` rather than `off`, since the class of problem that leaves a `warn` or an `error` behind is still answerable from the file as it stands.

## what the docs say

`AGENTS.md`, in `## Logs`, before:

```
The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards.
```

after:

```
The file records what the daemon's `--log-file-level` asked for, defaulting to `warn`: a daemon that is not being debugged keeps the reason it died and nothing routine. `mercury restart --log-file-level debug` is what makes a run reconstructable, and a bare `mercury restart` puts it back. The flag is read once, at startup, so it is a restart and never a running daemon that changes.
```

`AGENTS.md`, in `## Logs`, before:

```
`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file.
```

after:

```
`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury -- daemon --log-file-level debug` gives a quiet terminal and a full log file. It stays a variable rather than a flag because it filters a terminal, and the only invocation with one is a person typing in front of it; the file filter is a flag for the mirror-image reason, since the file is what a detached daemon and a launchd job write to.
```

`AGENTS.md`, in `## Logs`, before:

```
The daemon is different: its terminal is its log in full, filtered by `--log-level`.
```

after:

```
The daemon is different: its terminal is its log in full, filtered by `LOG_LEVEL`.
```

`AGENTS.md`, in `## Running mercury`, appended to the `mercury restart` line:

```
A daemon's settings are the flags it was started with, so `restart` with a flag is how one changes and a bare `restart` puts every one back to its default. `mercury install` takes the same flags and writes them into the launchd job, because that job starts with none of the environment `install` was run in; install again to change them.
```

`README.md`, in `## mercury logs`, before:

```
`mercury` writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and always down to `debug` whatever the terminal was asked for.
```

after:

```
`mercury` writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, at whatever `--log-file-level` asked for. That defaults to `warn`, so an ordinary run keeps problems and nothing else; `mercury restart --log-file-level debug` keeps everything, which is one record per dispatched event and one per keystroke.
```

## tests

`crates/freddie_cli`, added:

- `filter_from` returns the fallback and pushes one complaint for a directive that is not a filter, and the parsed filter with no complaint for one that is.
- `Verb::log_file_level` returns the flag for `start`, `restart`, and `daemon`, and `DEFAULT_LOG_FILE_LEVEL` for `status`, `logs`, and `stop`.
- `Inherits::Environment` re-emits a command-line value and drops an env-sourced one and a default; `Inherits::Nothing` re-emits all three. Driven through a `clap::Args` fixture with one flag carrying an `env` and a `default_value`, matched three ways.
- `--log-file-level` parses on `start`, `restart`, and `daemon`, and is refused on `status`, `logs`, and `stop`.

`crates/mercury`, added:

- `Agent::running` puts the flags after the verb, and a plist round-trips with them.
- `install` with no flags writes the defaults spelled out rather than an empty tail, since the job resolves nothing from the environment.

## changes

1. The flag and the file's filter: `DEFAULT_LOG_FILE_LEVEL`, `LOG_FILE_LEVEL`, `filter_from`, `log_file_level` on `DaemonVerbArgs`, `StartArgs`, and `RestartArgs`, `Verb::log_file_level`, `init`'s third parameter, and the removal of `FILE_LEVEL`. Independently shippable, and it is the whole of "file logging is opt-in".
2. `logs`: the `--level` doc, the `shown` count, and `NOTHING_KEPT`. Independently shippable, and worth landing with the first, since the first is what makes an empty follow ordinary.
3. `install` carries the flags: `Inherits`, `TypedArgs::argv`'s parameter and its `pub`, `spawn_daemon`'s doc, mercury's `InstallArgs`, `Agent::running`'s second parameter, and `install_agent`. Independently shippable. figaro's `src/cli/agent.rs` is the same file with the name changed and takes the same edit, in its own repo and its own commit.
4. The wording in `AGENTS.md` and `README.md`, in the same commits as the changes they describe.

`bounded-log.md` proposes the same setting as an environment variable read with `std::env::var`, with a `DEFAULT_FILE_LEVEL` of `"debug,keystroke=off"`. Change 1 here replaces that half of it: the setting is a flag whose env fallback clap resolves, and a default of `warn` drops the keystroke records without a directive naming them. The rest of that doc stands as written. `KEYSTROKE_TARGET` is still worth having, because it is what lets a daemon started at `debug` still mute the three sites that write a line per key, and change 2's rolling file is untouched by any of this.
