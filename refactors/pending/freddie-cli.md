# the command line is freddie's, the daemon is the app's

A freddie app is one process holding something there can only be one of, plus a handful of verbs for finding it, starting it, and stopping it. mercury owns the keyboard; isograph v2 watches a set of project directories. What the single process is for differs, and the verbs that manage it do not. All of them live in mercury today, where a second app cannot reach them.

`freddie_cli` is a new crate holding the whole command surface. An app supplies its name, its daemon body, and whatever extra flags that body takes; it gets `start`, `restart`, `status`, `logs`, `stop`, `install`, `uninstall`, and the hidden `daemon` for free, keyed to its own name and writing to its own log file. mercury becomes an implementation of one trait and a `main` that is a single call.

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
- what quitting means, which `freddie-daemon-runtime.md` routes to it as an abstract `Stop`

## The seam

```rust
/// What an app is, to the command line that runs it.
///
/// One impl per binary. Everything else in this crate is generic over it, so `A` names the app
/// wherever it appears and never one of the app's own types.
///
/// `Debug` because the derived `Debug` on every type generic over an app asks for it. An app is a
/// unit struct, so this is a derive on the marker.
pub trait App: fmt::Debug {
    /// The flags this app's daemon takes beyond the shared ones.
    ///
    /// [`NoArgs`] for an app that takes none.
    type Args: clap::Args + fmt::Debug;

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
    fn run(args: &Self::Args) -> i32;
}

/// The daemon flags of an app that adds none.
#[derive(clap::Args, Debug)]
pub struct NoArgs {}
```

`freddie-daemon-runtime.md` replaces `run` with an associated `Daemon` type once the process arrangement is a crate too, so an app writes no function to be the daemon at all. Until then `run` is the app's whole daemon.

## The generic command line

An app's daemon takes flags of its own: mercury's `--port` names the socket the extension talks to, and isograph's is a config file path. So the command line is generic over the app, and each of the types that carries the app's flags flattens `A::Args` in. clap's derive accepts generic parameters, which is what lets it. Verified on the pinned 1.96.0 against clap 4.6.2: a `Subcommand` enum whose variants are a mix of generic and not derives, an arg struct generic over `A: App` flattens the associated `A::Args`, `app daemon --port 4001` parses into it, the shared defaults resolve, and `NoArgs` gives an app with no flags of its own.

Four verbs are generic, and they are the four that run the daemon: `daemon` is it, `start` and `restart` spawn it, and `install` writes the agent that launchd starts it from. A flag the daemon takes has to reach it whichever of those put it there, so each of them takes the app's flags and hands them on. `status`, `logs`, `stop`, and `uninstall` only ever find a daemon that is already running, so they stay concrete, and their arg structs move across unchanged from `crates/mercury/src/cli.rs`.

```rust
#[derive(Parser, Debug)]
#[command(version, long_about = None)]
pub struct Args<A: App> {
    #[command(subcommand)]
    pub verb: Option<Verb<A>>,
}

/// What the command line asked the app to do, where `None` is the bare binary.
///
/// Each variant's doc comment is its line in `--help`, so the help text cannot drift from the
/// verbs. Declaration order is help order.
#[derive(Subcommand, Debug)]
pub enum Verb<A: App> {
    /// Start the daemon if it is not running, and exit.
    Start(StartArgs<A>),
    /// Stop the running daemon and start a fresh one.
    Restart(RestartArgs<A>),
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Follow the log, starting nothing.
    Logs(LogsArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
    /// Register this binary as a login agent, so the app starts with the session.
    Install(InstallArgs<A>),
    /// Take the login agent back out.
    Uninstall,
    /// Run the daemon in this process. Not for typing: `start` spawns it.
    #[command(hide = true)]
    Daemon(DaemonArgs<A>),
}

/// What the foreground daemon can be told: what this crate asks of every app, and what the app
/// asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonArgs<A: App> {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on.
    #[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]
    pub log_level: String,

    #[command(flatten)]
    pub app: A::Args,
}

/// What `start` can be told, which is whatever the daemon it spawns can be told.
///
/// No `--log-level`: the daemon it spawns has no terminal to show anything on.
#[derive(clap::Args, Debug)]
pub struct StartArgs<A: App> {
    #[command(flatten)]
    pub app: A::Args,
}

/// What `restart` can be told: `start`'s flags, and how hard to stop what is running.
#[derive(clap::Args, Debug)]
pub struct RestartArgs<A: App> {
    /// Destroy the running daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so whatever the app undoes on
    /// the way out is left undone.
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub app: A::Args,
}

/// What `install` can be told, which is what the agent it writes will start the daemon with.
#[derive(clap::Args, Debug)]
pub struct InstallArgs<A: App> {
    #[command(flatten)]
    pub app: A::Args,
}
```

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
        Some(Verb::Install(_)) => client::install::<A>(typed),
        Some(Verb::Uninstall) => client::uninstall::<A>(),
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

`start` spawns the daemon, `restart` spawns a replacement, and `install` writes the agent launchd spawns it from. All three have to give it the flags this invocation was given, and none of them can know what those flags are: `A::Args` is the app's.

clap parses argv into a struct and does not write one back out, so the flags are re-emitted from the matches instead. Reading the same derive that parsed them is what keeps this from drifting: an app that adds a flag gets it forwarded without writing a line.

```rust
/// The app flags this invocation typed, for the process that will run the daemon.
///
/// `None` is the bare binary, which typed nothing. Borrowed from the matches rather than parsed
/// out of them, because what has to be re-emitted is what was written, not what it resolved to.
#[derive(Clone, Copy)]
pub(crate) struct Typed<'a>(Option<&'a ArgMatches>);

/// Where the daemon is about to be started, which decides what its flags have to carry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Started {
    /// By this process, as a child. It inherits this environment, so anything the environment
    /// said is already there.
    AsChild,
    /// By launchd, from the agent `install` writes. The session it starts in never saw the shell
    /// that set these, so what the environment said has to be written into the plist.
    AtLogin,
}

impl Typed<'_> {
    /// Re-emit `A`'s flags as argv for the process named by `started`.
    ///
    /// A default is left out either way: the daemon resolves the same one. A value that came from
    /// the environment is left out for a child, which inherits it, and written in for the agent,
    /// which does not.
    pub(crate) fn argv<A: App>(self, started: Started) -> Vec<OsString> {
        let Some(matches) = self.0 else {
            return Vec::new();
        };
        let mut argv = Vec::new();
        for arg in A::Args::augment_args(Command::new(A::NAME)).get_arguments() {
            let id = arg.get_id().as_str();
            let carried = match matches.value_source(id) {
                Some(ValueSource::CommandLine) => true,
                Some(ValueSource::EnvVariable) => started == Started::AtLogin,
                _ => false,
            };
            // A positional has no flag to re-emit it under. `A::Args` is a set of flags: the verbs
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

Verified on the pinned 1.96.0 against clap 4.6.2, against an `A::Args` of `--port` (with an env and a default), `--config` (an optional path), and `--verbose` (a flag):

- `start --port 4001 --config "/a b/c.toml"` re-emits exactly `["--port", "4001", "--config", "/a b/c.toml"]`. The value with a space needs no quoting, because these reach `Command::arg` as separate `OsString`s and no shell parses them.
- `restart --force --verbose` re-emits `["--verbose"]`. The verb's own `--force` drops out with no stripping, because it is not one of `A::Args`'s arguments.
- `start` with nothing typed re-emits nothing, and `MERCURY_PORT=5005 start` re-emits nothing for a child and `["--port", "5005"]` for the agent, having parsed as `5005` in both.
- `NoArgs` re-emits nothing.

A path an app takes as a flag is the app's to make absolute. launchd starts the agent in a directory the shell never chose, so a relative one recorded at `install` names somewhere else at login.

`use clap::builder::ArgAction; use clap::parser::ValueSource; use clap::{ArgMatches, Args as _, Command};`.

Then the two places that build a daemon's argv take it. `spawn_daemon`, before:

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
        .args(typed.argv::<A>(Started::AsChild))
```

and `Agent::running`, before:

```rust
            program_arguments: vec![program.to_string_lossy().into_owned(), "daemon".to_owned()],
```

after:

```rust
            program_arguments: std::iter::once(program.as_os_str().to_owned())
                .chain(std::iter::once(OsString::from("daemon")))
                .chain(typed.argv::<A>(Started::AtLogin))
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
```

`--log-level` is forwarded by neither, and for the reason `spawn_daemon` already gives: it governs a terminal that a detached child and a launchd agent both lack.

## The daemon verb

The lock and the logging move ahead of the app, so an app cannot forget either and two apps cannot disagree about where they go.

```rust
/// Run `A`'s daemon in the foreground: set up logging, take the lock, and hand over.
pub(crate) fn run<A: App>(args: &DaemonArgs<A>) -> i32 {
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

The daemon verb reports a code where mercury's exits 0 whatever happened: a lock it could not take and a menu bar it could not create are both a daemon that did not run, and `install`'s launch agent is tuned to the exit code.

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

`crates/mercury/src/client.rs` becomes `freddie_cli`'s `client` module. No verb changes what it does, what it prints, or what it exits with. Four things change throughout:

- `const APP: &str = "mercury"` is deleted, and each of its seven use sites becomes `A::NAME`.
- Every function that reads the name, or calls one that does, gains `<A: App>`.
- Every message that spells "mercury" spells `A::NAME`, which is what makes a fork's output its own.
- `start`, `restart`, and `install` take a `Typed<'_>` and pass it down to whichever of `spawn_daemon` and `Agent::running` they reach: `start<A: App>(typed: Typed<'_>)`, `restart<A: App>(args: &RestartArgs<A>, typed: Typed<'_>)`, `install<A: App>(typed: Typed<'_>)`, and `ensure_started<A: App>(typed: Typed<'_>)` between the first two and the spawn. The other four verbs take no such thing, since none of them starts a daemon.

The name reaches the lock:

```rust
fn ensure_started<A: App>() -> Result<Running, NotStarted> {
    match freddie_single_instance::holder(A::NAME) {
```

the launchd label:

```rust
/// The launchd job's name, keyed to the same name as the lock and the log directory, so a fork
/// gets its own job rather than fighting this one over a label.
fn label<A: App>() -> String {
    format!("{AGENT_PREFIX}{}", A::NAME)
}

/// The reverse-DNS prefix every freddie app's launch agent sits under. The name after it is the
/// app's, so two of them cannot collide.
const AGENT_PREFIX: &str = "hg.freddie.";
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
#[derive(Debug)]
struct TestApp;

#[cfg(test)]
impl App for TestApp {
    type Args = NoArgs;
    const NAME: &'static str = "testapp";
    const ABOUT: &'static str = "A test app.";
    fn run(_: &Self::Args) -> i32 {
        unreachable!("the parse tests never run the daemon")
    }
}
```

`label`'s test asserts `hg.freddie.testapp`; `cli.rs`'s parse tests become `Args::<TestApp>::try_parse_from`, and the two that assert a port move to mercury, which is the crate that has one.

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

#[derive(Debug)]
struct Mercury;

impl App for Mercury {
    type Args = MercuryArgs;
    const NAME: &'static str = "mercury";
    const ABOUT: &'static str = "A layered keyboard remapper.";

    fn run(args: &Self::Args) -> i32 {
        daemon::run(args.port)
    }
}

fn main() -> ! {
    freddie_cli::main::<Mercury>()
}
```

`crates/mercury/src/cli.rs`, `crates/mercury/src/client.rs`, and `crates/mercury/src/logging.rs` are deleted; their contents live in `freddie_cli`.

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

`freddie_cli`'s dependencies are the ones those three files already pull in: `clap`, `tracing`, `tracing-subscriber`, `tracing-appender`, `serde` and `plist` for the launch agent, and `freddie_single_instance`. mercury drops `tracing-subscriber`, `tracing-appender`, `plist`, and `freddie_single_instance`, and keeps `clap` for its own `Args` derive, `tracing` for the daemon, and `serde` for the wire types in `external.rs`.

## The changes, in order

1. **Logging takes a name.** `log_dir(name)`, the file name derived from it, and `init(name, terminal)`, in mercury, with `client::APP` passed at each of the four call sites. No behaviour changes: the name passed is the name that was written.
2. **`freddie_cli`, holding the whole command surface.** The trait, the generic `Args`/`Verb`/`DaemonArgs`, `parse`, `main`, `dispatch`, the daemon verb, `logging`, and the `client` module, all keyed to `A::NAME`. mercury shrinks to the `main.rs` above and a `daemon.rs` that starts at `freddie_windows::init()`. Every verb does what it does now, and `mercury --help` prints what it prints now.

One change rather than one per verb: `Verb` is a single enum and `dispatch` a single match, so a `freddie_cli` holding some of the verbs would leave mercury without the rest.
