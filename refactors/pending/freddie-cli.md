# the command line is freddie's, the daemon is the app's

An app built here is one process holding something there can only be one of, plus a handful of verbs for finding it, starting it, and stopping it. mercury owns the keyboard; isograph v2 watches a set of project directories. What the single process is for differs, and the verbs that manage it do not. All of them live in mercury today, where a second app cannot reach them.

The verbs are the same because none of them looks inside the process. They read a lock file, spawn a binary, send a signal, and tail a log, and every one of those works the same for a program that has nothing to do with keys or events.

`freddie_cli` is a new crate holding the whole command surface. An app supplies its name, its daemon body, and whatever extra flags that body takes; it gets `start`, `restart`, `status`, `logs`, `stop`, and the hidden `daemon` for free, keyed to its own name and writing to its own log file. mercury becomes an implementation of one trait and a `main` that is a single call.

The name of the binary is the app's, not mercury's. Nothing in `freddie_cli` spells "mercury", and nothing in an app spells the verbs.

A new crate rather than a dependency, and that is settled rather than assumed: `refactors/past/reuse-existing-crates.md` audited `single-instance`, `service-manager`, and `daemonize` against what these verbs need and none of them fit. `single-instance` cannot probe without acquiring or report a pid, so `status` and `stop` have nothing to build on; `service-manager` cannot express `SuccessfulExit=false`, which is the key the daemon's exit code is tuned to. Whoever forks this gets the lifecycle from here or writes it again themselves.

## What belongs where

`freddie_cli` owns:

- the `Args`/`Verb` types and the parse
- the single-instance lock: acquiring it for the daemon, probing it for the clients
- logging setup, and the log directory
- every client verb, since all of them are the lock, the log file, and a subprocess
- delivering SIGTERM to the app as a request to stop

The app owns:

- its name, which keys the lock file, the log directory, the launchd label, the help text, and every line a verb prints
- what the daemon does, which is everything `mercury::daemon` holds today
- any flag beyond `--log-level` that its daemon takes
- what quitting means, since this crate only ever sends a signal and waits for the lock to go free
- whether it installs itself to start with the session, and what that job says

## The seam

```rust
/// What an app is, to the command line that runs it.
///
/// One impl per binary. Everything else in this crate is generic over it, so `A` names the app
/// wherever it appears and never one of the app's own types.
pub trait App {
    /// The flags this app's daemon takes beyond the shared ones.
    ///
    /// [`NoArgs`] for an app that takes none.
    type DaemonArgs: clap::Args + fmt::Debug;

    /// The name the lock file, the log directory, the launchd label, `--help`, and every message a
    /// verb writes are all keyed to.
    ///
    /// One name, so a client cannot look for the daemon under a name the daemon did not
    /// register under.
    const NAME: &'static str;

    /// The one-line description at the top of `--help`.
    const ABOUT: &'static str;

    /// Be the daemon. Returns the process's exit code when it has quit.
    ///
    /// Called with the lock held and logging initialized. Returning drops the lock, so an app
    /// that wants to stay running stays inside this call.
    fn run(args: &Self::DaemonArgs) -> i32;
}

/// The daemon flags of an app that adds none.
#[derive(clap::Args, Debug)]
pub struct NoArgs {}
```

That is the whole trait, and nothing in it is about freddie. A name, an about line, some flags, and a function that runs until it is done: any program that wants one instance of itself and the verbs to manage it fits, whether or not it has a model, an event, or a keyboard. `freddie-daemon-runtime.md` is what mercury calls inside `run`, and this crate never learns that it exists.

## The generic command line

An app's daemon takes flags of its own: mercury's `--port` names the socket the extension talks to, and isograph's is a config file path. So the command line is generic over the app, and each of the types that carries the app's flags flattens `A::DaemonArgs` in. clap's derive accepts generic parameters, which is what lets it. Verified on the pinned 1.96.0 against clap 4.6.2: a `Subcommand` enum whose variants are a mix of generic and not derives, an arg struct generic over the flags flattens them in, `app daemon --port 4001` parses into it, the shared defaults resolve, and `NoArgs` gives an app with no flags of its own.

Three verbs are generic, and they are the three that start the daemon: `daemon` is it, and `start` and `restart` spawn it. A flag the daemon takes has to reach it whichever of those put it there, so each of them takes the app's flags and hands them on. `status`, `logs`, and `stop` only ever find a daemon that is already running, so they stay concrete, and their arg structs move across unchanged from `crates/mercury/src/cli.rs`.

```rust
/// No `Debug` derive: `Args` and `Verb` hold no `A`, but the derive would ask every app for one
/// anyway. The types that do hold flags derive it below, where the bound is real.
#[derive(Parser)]
#[command(version, long_about = None)]
pub struct Args<A: App> {
    #[command(subcommand)]
    pub verb: Option<Verb<A>>,
}

/// What the command line asked the app to do, where `None` is the bare binary.
///
/// Each variant's doc comment is its line in `--help`, so the help text cannot drift from the
/// verbs. Declaration order is help order.
#[derive(Subcommand)]
pub enum Verb<A: App> {
    /// Start the daemon if it is not running, and exit.
    Start(StartArgs<A::DaemonArgs>),
    /// Stop the running daemon and start a fresh one.
    Restart(RestartArgs<A::DaemonArgs>),
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Follow the log, starting nothing.
    Logs(LogsArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
    /// Run the daemon in this process. Not for typing: `start` spawns it.
    #[command(hide = true)]
    Daemon(DaemonVerbArgs<A::DaemonArgs>),
}

/// What the foreground daemon can be told: what this crate asks of every app, and what the app
/// asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<F: clap::Args> {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on.
    #[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]
    pub log_level: String,

    #[command(flatten)]
    pub app: F,
}

/// What `start` can be told, which is whatever the daemon it spawns can be told.
///
/// No `--log-level`: the daemon it spawns has no terminal to show anything on.
#[derive(clap::Args, Debug)]
pub struct StartArgs<F: clap::Args> {
    #[command(flatten)]
    pub app: F,
}

/// What `restart` can be told: `start`'s flags, and how hard to stop what is running.
#[derive(clap::Args, Debug)]
pub struct RestartArgs<F: clap::Args> {
    /// Destroy the running daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so whatever the app undoes on
    /// the way out is left undone.
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub app: F,
}
```

The arg structs take the flags rather than the app, so their `Debug` derives against `F: Debug`, which the associated type satisfies, and no app is asked for a `Debug` it has no use for.

The `Start` and `Daemon` doc comments lose the binary's name, since the derive writes them into every app's `--help`. `LogsArgs` moves across as it is; `RestartArgs` gains the app's flags and keeps `--force`.

`StopArgs` and `RestartArgs` say what `--force` costs, and `Signal`'s two variants say what each signal does. All four of those doc comments are written about mercury today, naming the keyboard grab and the modifiers a command layer swallowed. They are reworded down to what this crate actually knows, which is a process, its pid, a lock, and two signals: SIGTERM asks the daemon to quit and it leaves the way it chose to, SIGKILL destroys it so no destructor runs and whatever it would have undone stays undone. Nothing here knows there is a model, an event, or a keyboard. A doc comment is the one place "mercury" can survive the move without the compiler noticing.

`#[command(name = ...)]` cannot carry `A::NAME`, because a derive macro reads only what is written beside it. The name is set on the built `Command` instead, which is also where `A::ABOUT` goes. Verified on the pinned 1.96.0: `--help` prints `Usage: mercury [COMMAND]` with the app's about line above it.

`use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};`.

## The entry point

`dispatch` takes the matches beside the verb, because the three verbs that put a daemon somewhere have to hand it the flags this invocation was given, and that is read off the matches rather than off the parsed struct. `Typed` is what carries them.

```rust
/// Do what the command line asked, and report the exit code for it.
fn dispatch<A: App>(verb: Option<Verb<A>>, typed: Typed<'_>) -> i32 {
    match verb {
        // The bare invocation is `start`: one behaviour, not two that agree.
        None => client::start::<A>(typed),
        Some(Verb::Start(_)) => client::start::<A>(typed),
        Some(Verb::Restart(args)) => client::restart::<A>(&args, typed),
        Some(Verb::Status) => client::status::<A>(),
        Some(Verb::Logs(args)) => client::logs::<A>(&args),
        Some(Verb::Stop(args)) => client::stop::<A>(&args),
        Some(Verb::Daemon(args)) => daemon::run::<A>(&args),
    }
}

/// Be `A`'s command line. Never returns.
///
/// `Command::name` at runtime rather than `#[command(name)]` at derive time: the name belongs to
/// the app, and the derive cannot see a constant from a type it is generic over.
///
/// # Panics
///
/// Panics if the derived parser and the derived command disagree, which is a bug in this crate
/// rather than anything a caller can cause.
pub fn main<A: App>() -> ! {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    let command = Args::<A>::command().name(A::NAME).about(A::ABOUT);
    let matches = command.get_matches();
    let args = Args::<A>::from_arg_matches(&matches)
        .expect("the derived type matches the command it derived");

    // The verb's own matches, where its flattened app flags live. No verb is the bare binary,
    // which typed no flags at all. Taken through `subcommand` so no verb's name is spelled here.
    let typed = Typed(matches.subcommand().map(|(_name, verb)| verb));

    let code = dispatch::<A>(args.verb, typed);
    // Every path above has returned, so the daemon's locals are already dropped by the time this
    // skips the rest of the destructors.
    std::process::exit(code);
}
```

## Handing the app's flags to the daemon

`start` spawns the daemon and `restart` spawns a replacement. Both have to give it the flags this invocation was given, and neither can know what those flags are: `A::DaemonArgs` is the app's.

clap parses argv into a struct and does not write one back out, so the flags are re-emitted from the matches instead. Reading the same derive that parsed them is what keeps this from drifting: an app that adds a flag gets it forwarded without writing a line.

```rust
/// The app flags this invocation typed, for the process that will run the daemon.
///
/// `None` is the bare binary, which typed nothing. Borrowed from the matches rather than parsed
/// out of them, because what has to be re-emitted is what was written, not what it resolved to.
#[derive(Clone, Copy)]
pub(crate) struct Typed<'a>(Option<&'a ArgMatches>);

impl Typed<'_> {
    /// Re-emit `A`'s flags as argv for the daemon this process is about to spawn.
    ///
    /// Only what was typed. A default is left out because the daemon resolves the same one, and a
    /// value from the environment is left out because the child inherits the environment it came
    /// from and resolves that too.
    pub(crate) fn argv<A: App>(self) -> Vec<OsString> {
        let Some(matches) = self.0 else {
            return Vec::new();
        };
        let mut argv = Vec::new();
        for arg in A::DaemonArgs::augment_args(Command::new(A::NAME)).get_arguments() {
            let id = arg.get_id().as_str();
            let carried = matches.value_source(id) == Some(ValueSource::CommandLine);
            // A positional has no flag to re-emit it under. `A::DaemonArgs` is a set of flags: the verbs
            // own their positionals, and an app's positional would be ambiguous against them.
            let Some(long) = arg.get_long().filter(|_| carried) else {
                continue;
            };
            let flag = OsString::from(format!("--{long}"));
            match arg.get_action() {
                ArgAction::SetTrue | ArgAction::SetFalse => argv.push(flag),
                _ => {
                    for value in matches.try_get_raw(id).ok().flatten().into_iter().flatten() {
                        argv.push(flag.clone());
                        argv.push(value.to_owned());
                    }
                }
            }
        }
        argv
    }
}
```

Verified on the pinned 1.96.0 against clap 4.6.2, against an `A::DaemonArgs` of `--port` (with an env and a default), `--config` (an optional path), and `--verbose` (a flag):

- `start --port 4001 --config "/a b/c.toml"` re-emits exactly `["--port", "4001", "--config", "/a b/c.toml"]`. The value with a space needs no quoting, because these reach `Command::arg` as separate `OsString`s and no shell parses them.
- `restart --force --verbose` re-emits `["--verbose"]`. The verb's own `--force` drops out with no stripping, because it is not one of `A::DaemonArgs`'s arguments.
- `start` with nothing typed re-emits nothing, and so does `MERCURY_PORT=5005 start`, having parsed as `5005` either way. The child inherits `MERCURY_PORT` and reaches `5005` on its own.
- `NoArgs` re-emits nothing.

`use clap::builder::ArgAction; use clap::parser::ValueSource; use clap::{ArgMatches, Args as _, Command};`.

Then `spawn_daemon` takes it. Before:

```rust
fn spawn_daemon() -> io::Result<Pid> {
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("daemon")
```

after:

```rust
fn spawn_daemon<A: App>(typed: Typed<'_>) -> io::Result<Pid> {
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("daemon")
        .args(typed.argv::<A>())
```

`--log-level` is not forwarded, and for the reason `spawn_daemon` already gives: it governs a terminal a detached child does not have.

The hidden verb's name is spelled here and nowhere else, so an app writing a launch agent has something to point at rather than a string of its own:

```rust
/// What the daemon verb is called, for an app building an argv that has to reach it.
pub const DAEMON_VERB: &str = "daemon";
```

## The daemon verb

The lock and the logging move ahead of the app, so an app cannot forget either and two apps cannot disagree about where they go.

```rust
/// Run `A`'s daemon in the foreground: set up logging, take the lock, and hand over.
pub(crate) fn run<A: App>(args: &DaemonVerbArgs<A::DaemonArgs>) -> i32 {
    let log_path = logging::init(A::NAME, &Terminal::Daemon(LogLevel(&args.log_level)));
    info!(path = %log_path.display(), "logging");

    // Before anything that touches the machine. Two instances swallow and re-emit each other's
    // keys forever, which wedges the keyboard. The binding must outlive the call (`let _instance`,
    // never `let _`): dropping it releases the lock.
    let _instance = match freddie_single_instance::acquire(A::NAME) {
        Ok(instance) => instance,
        Err(e) => {
            error!(name = A::NAME, error = %e, "another instance holds the lock; `stop` ends it");
            return 1;
        }
    };

    A::run(&args.app)
}
```

The daemon verb reports a code where mercury's exits 0 whatever happened: a lock it could not take and a menu bar it could not create are both a daemon that did not run. An app whose launch agent declines to revive a clean exit depends on this, which mercury's does.

## Logging keyed to the name

`crates/mercury/src/logging.rs` moves to `freddie_cli` and takes the name it currently spells twice. Its `Terminal`/`LogLevel` split, its pid stamp, its file level, and its client layers cross unchanged.

Before:

```rust
/// The log file, written under [`log_dir`].
const LOG_FILE: &str = "mercury.log";

fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Logs/mercury"),
    )
}

pub fn init(terminal: &Terminal<'_>) -> PathBuf {
    let dir = log_dir();
```

After:

```rust
/// Where the log file lives: the macOS per-user log directory, or the current
/// directory when `HOME` is unset.
fn log_dir(name: &str) -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Logs").join(name),
    )
}

pub fn init(name: &str, terminal: &Terminal<'_>) -> PathBuf {
    let dir = log_dir(name);
    let file_name = format!("{name}.log");
```

`LOG_FILE` is deleted; `rolling::never(&dir, &file_name)` and `dir.join(file_name)` take its place. The one message `init` writes itself is keyed the same way:

```rust
    for problem in setup {
        warn!("{name}: {problem}");
    }
```

## The client verbs

`crates/mercury/src/client.rs` becomes `freddie_cli`'s `client` module, apart from what `install` and `uninstall` need: `Agent`, `KeepAlive`, `label`, `plist_path`, `domain`, `bootout`, `launchctl`, `users_uid`, `NotInstalled`, and the `TRANSIENT` and `ID` constants all stay in mercury. A launch agent says which session type it loads in, when launchd should revive it, and what its label is, and those are answers about one app rather than about daemons. `app-verbs.md` is what mercury reaches those two verbs through.

What does move is every verb that only reads a lock, spawns a binary, signals a pid, or tails a log. No verb changes what it does, what it prints, or what it exits with. Four things change throughout:

- `const APP: &str = "mercury"` is deleted. Five of its use sites move here and become `A::NAME`; the two under `label` stay in mercury with the launch agent.
- Every function that reads the name, or calls one that does, gains `<A: App>`.
- Every message that spells "mercury" spells `A::NAME`, which is what makes a fork's output its own.
- `start` and `restart` take a `Typed<'_>` and pass it down to `spawn_daemon`: `start<A: App>(typed: Typed<'_>)`, `restart<A: App>(args: &RestartArgs<A::DaemonArgs>, typed: Typed<'_>)`, and `ensure_started<A: App>(typed: Typed<'_>)` between them and the spawn. `status`, `logs`, and `stop` take no such thing, since none of them starts a daemon.

The name reaches the lock:

```rust
fn ensure_started<A: App>() -> Result<Running, NotStarted> {
    match freddie_single_instance::holder(A::NAME) {
```

and every line a verb writes:

```rust
pub(crate) fn status<A: App>() -> i32 {
    logging::init(A::NAME, &Terminal::Client);
    match freddie_single_instance::holder(A::NAME) {
        Ok(Held::Free) => {
            info!("{} is not running", A::NAME);
            1
        }
        Ok(Held::By(pid)) => {
            info!("{} is running (pid {pid})", A::NAME);
            0
        }
```

The tests move with the module and pick the name up from a test app:

```rust
#[cfg(test)]
struct TestApp;

#[cfg(test)]
impl App for TestApp {
    type DaemonArgs = NoArgs;
    const NAME: &'static str = "testapp";
    const ABOUT: &'static str = "A test app.";
    fn run(_: &Self::DaemonArgs) -> i32 {
        unreachable!("the parse tests never run the daemon")
    }
}
```

`cli.rs`'s parse tests become `Args::<TestApp>::try_parse_from`, and the two that assert a port move to mercury, which is the crate that has one.

## What mercury becomes

`crates/mercury/src/main.rs`, entire:

```rust
//! The mercury binary: a freddie app and the command line that runs it.

use freddie_cli::App;

mod daemon;

/// The loopback port the event socket listens on, and nothing else mercury needs from a flag.
#[derive(clap::Args, Debug)]
pub struct MercuryArgs {
    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}

struct Mercury;

impl App for Mercury {
    type DaemonArgs = MercuryArgs;
    // `install` and `uninstall`, which are mercury's own. See `app-verbs.md`.
    type Subcommands = MercuryVerb;

    const NAME: &'static str = "mercury";
    const ABOUT: &'static str = "A layered keyboard remapper.";

    fn run(args: &Self::DaemonArgs) -> i32 {
        daemon::run(args.port)
    }

    fn run_subcommand(verb: MercuryVerb) -> i32 {
        agent::run(verb)
    }
}

fn main() -> ! {
    freddie_cli::main::<Mercury>()
}
```

`crates/mercury/src/cli.rs` and `crates/mercury/src/logging.rs` are deleted; their contents live in `freddie_cli`. `crates/mercury/src/client.rs` becomes `agent.rs`, holding only what `install` and `uninstall` need.

`crates/mercury/src/daemon.rs` keeps everything it holds today except the logging call and the lock, which `freddie_cli` does before calling it. Before:

```rust
pub(crate) fn run(args: &DaemonArgs) {
    let log_path = logging::init(&Terminal::Daemon(LogLevel(&args.log_level)));
    info!(path = %log_path.display(), "logging");

    let _instance = match freddie_single_instance::acquire(crate::client::APP) {
        Ok(instance) => instance,
        Err(e) => {
            error!(error = %e, "another mercury holds the lock; `mercury stop` ends it");
            return;
        }
    };

    if let Err(e) = freddie_windows::init() {
```

After:

```rust
pub(crate) fn run(port: u16) -> i32 {
    if let Err(e) = freddie_windows::init() {
```

Its two early returns become `1` and its end becomes `0`, so a menu bar that could not be created is a daemon that did not run. `use crate::cli::DaemonArgs;` and `use crate::logging::{self, LogLevel, Terminal};` go with the modules they name.

`freddie_cli`'s dependencies are `clap`, `tracing`, `tracing-subscriber`, `tracing-appender`, and `freddie_single_instance`. mercury drops `tracing-subscriber` and `tracing-appender`, and keeps `clap`, `tracing`, `serde` for the wire types and the launch agent, `plist` for writing it, and `freddie_single_instance`, which `label` and the agent verbs still reach.

## The changes, in order

1. **Logging takes a name.** `log_dir(name)`, the file name derived from it, and `init(name, terminal)`, in mercury, with `client::APP` passed at each of the four call sites. No behaviour changes: the name passed is the name that was written.
2. **`freddie_cli`, holding the command surface, and `app-verbs.md` with it.** The trait, the generic `Args`/`Verb`/`DaemonVerbArgs`, `main`, `dispatch`, the daemon verb, `logging`, and the `client` module, all keyed to `A::NAME`. mercury shrinks to the `main.rs` above, a `daemon.rs` that starts at `freddie_windows::init()`, and an `agent.rs` holding `install` and `uninstall`. Every verb does what it does now, and `mercury --help` prints what it prints now.

`app-verbs.md` lands here rather than after, because `install` and `uninstall` are mercury's own verbs and there is no way to keep them without the seam that carries them.

One change rather than one per verb: `Verb` is a single enum and `dispatch` a single match, so a `freddie_cli` holding some of the verbs would leave mercury without the rest.
