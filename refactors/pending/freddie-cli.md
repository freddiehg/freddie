# the command line is freddie's, the daemon is the app's

An app built here is a process holding something there can only be one of, plus a handful of verbs for finding it, starting it, and stopping it. mercury owns the keyboard, and there is one of it on the machine. isograph v2 watches the directories one config names, and there is one of it per config, so a machine runs as many as you have projects open. What the process is for differs, how many there are differs, and the verbs that manage one do not. All of them live in mercury today, where a second app cannot reach them.

The verbs are the same because none of them looks inside the process. They read a lock file, spawn a binary, send a signal, and tail a log, and every one of those works the same for a program that has nothing to do with keys or events.

`freddie_cli` is a new crate holding the whole command surface. An app supplies its name, what names one of its daemons, its daemon body, and whatever extra flags that body takes; it gets `start`, `restart`, `status`, `logs`, `stop`, and the hidden `daemon` for free, each keyed to the daemon the command line named and writing to that daemon's own log file. mercury becomes an implementation of one trait and a `main` that is a single call.

The name of the binary is the app's, not mercury's. Nothing in `freddie_cli` spells "mercury", and nothing in an app spells the verbs.

A new crate rather than a dependency, and that is settled rather than assumed: `refactors/past/reuse-existing-crates.md` audited `single-instance`, `service-manager`, and `daemonize` against what these verbs need and none of them fit. `single-instance` cannot probe without acquiring or report a pid, so `status` and `stop` have nothing to build on; `service-manager` cannot express `SuccessfulExit=false`, which is the key the daemon's exit code is tuned to. Whoever forks this gets the lifecycle from here or writes it again themselves.

## What belongs where

`freddie_cli` owns:

- the `Args`/`Verb` types and the parse
- the single-instance lock: acquiring it for the daemon, probing it for the clients, both under the instance the command line named
- logging setup, and the log directory
- every client verb, since all of them are the lock, the log file, and a subprocess
- delivering SIGTERM to the app as a request to stop

The app owns:

- its name, which keys the log directory and the help text
- what names one of its daemons, and how many of them there can be
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
    /// What names one of this app's daemons, as flags every verb takes.
    ///
    /// [`NoArgs`] for an app with one global daemon, which every verb then means without saying
    /// so. isograph's is its `--config`, so `isograph status --config ./a.json` asks about one
    /// daemon and `--config ./b.json` asks about another.
    type Id: clap::Args + fmt::Debug;

    /// The flags this app's daemon takes beyond the shared ones and beyond [`Id`](Self::Id).
    ///
    /// [`NoArgs`] for an app that takes none. Separate from `Id` so the verbs that only find a
    /// daemon take only what names one: `isograph status --port 4001` is refused rather than
    /// accepted and ignored.
    type DaemonArgs: clap::Args + fmt::Debug;

    /// The name of the binary, which keys the log directory and `--help`.
    const NAME: &'static str;

    /// The one-line description at the top of `--help`.
    const ABOUT: &'static str;

    /// Which daemon the command line named, or `None` if it named none that could exist.
    ///
    /// Fallible and allowed to touch the filesystem, because naming a daemon can mean reading
    /// one. isograph's id comes from the config, not from the text of the flag: `./foo.json` and
    /// `/abs/foo.json` are one daemon, so it resolves the path before it keys anything to it, and
    /// a config that is not there names no daemon at all. Say why in the log before returning
    /// `None`; the caller knows only that there is nothing to act on and exits 1.
    ///
    /// Called before the lock, the log file, and any subprocess, so every verb agrees on which
    /// daemon it meant before it does anything.
    fn instance(id: &Self::Id) -> Option<Instance>;

    /// Be the daemon. Returns the process's exit code when it has quit.
    ///
    /// Called with the lock held and logging initialized. Returning drops the lock, so an app
    /// that wants to stay running stays inside this call.
    fn run(id: &Self::Id, args: &Self::DaemonArgs) -> i32;
}

/// The daemon flags, or the id, of an app that has none.
#[derive(clap::Args, Debug)]
pub struct NoArgs {}
```

One daemon of one app: what its lock is under, what its log is called, and what a verb calls it when it says something about it.

```rust
/// Which daemon this is.
///
/// Two strings rather than one, because they answer to different things. A lock file and a log
/// file have to be named something the filesystem will take, and a person reading `status` has to
/// recognize what they typed. For an app with one global daemon both are its name.
#[derive(Clone, Debug)]
pub struct Instance {
    key: String,
    label: String,
}

impl Instance {
    /// The one daemon of an app that has one.
    pub fn global(name: &str) -> Self {
        Self { key: name.to_owned(), label: name.to_owned() }
    }

    /// One of many: `key` names its files, `label` is what the person who asked for it typed.
    ///
    /// `key` has to be stable across two invocations that mean the same daemon, and distinct for
    /// two that do not, since it is the whole of what the lock file is keyed to. A path that has
    /// been resolved, or a hash of one, is the shape of it.
    pub fn named(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into() }
    }
}
```

That is the whole trait, and nothing in it is about freddie. A name, an about line, some flags, and a function that runs until it is done: any program that wants one instance of itself and the verbs to manage it fits, whether or not it has a model, an event, or a keyboard. `freddie-daemon-runtime.md` is what mercury calls inside `run`, and this crate never learns that it exists.

## The generic command line

An app's daemon takes flags of its own: mercury's `--port` names the socket the extension talks to, and isograph's is a config file path. So the command line is generic over the app, and each of the types that carries the app's flags flattens `A::DaemonArgs` in. clap's derive accepts generic parameters, which is what lets it. Verified on the pinned 1.96.0 against clap 4.6.2: a `Subcommand` enum whose variants are a mix of generic and not derives, an arg struct generic over the flags flattens them in, `app daemon --port 4001` parses into it, the shared defaults resolve, and `NoArgs` gives an app with no flags of its own.

Every verb is generic, because every one of them is about one daemon and has to be told which. `status`, `logs`, and `stop` take `A::Id` and nothing more, since finding a daemon needs only its name. `daemon`, `start`, and `restart` take `A::DaemonArgs` as well, because they are the three that put a daemon somewhere and it needs its flags to run.

For mercury, whose `Id` is `NoArgs`, this adds nothing to any verb: `mercury status` takes no flags today and takes none after.

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
    Start(StartArgs<A::Id, A::DaemonArgs>),
    /// Stop the running daemon and start a fresh one.
    Restart(RestartArgs<A::Id, A::DaemonArgs>),
    /// Report whether the daemon is running, and its pid.
    Status(IdArgs<A::Id>),
    /// Follow the log, starting nothing.
    Logs(LogsArgs<A::Id>),
    /// Ask the running daemon to quit.
    Stop(StopArgs<A::Id>),
    /// Run the daemon in this process. Not for typing: `start` spawns it.
    #[command(hide = true)]
    Daemon(DaemonVerbArgs<A::Id, A::DaemonArgs>),
}

/// What the foreground daemon can be told: what this crate asks of every app, and what the app
/// asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<I: clap::Args, F: clap::Args> {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on.
    #[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]
    pub log_level: String,

    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}

/// What `start` can be told, which is whatever the daemon it spawns can be told.
///
/// No `--log-level`: the daemon it spawns has no terminal to show anything on.
#[derive(clap::Args, Debug)]
pub struct StartArgs<I: clap::Args, F: clap::Args> {
    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}

/// What `restart` can be told: `start`'s flags, and how hard to stop what is running.
#[derive(clap::Args, Debug)]
pub struct RestartArgs<I: clap::Args, F: clap::Args> {
    /// Destroy the running daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so whatever the app undoes on
    /// the way out is left undone.
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}

/// What a verb that only finds a daemon can be told, which is which daemon.
#[derive(clap::Args, Debug)]
pub struct IdArgs<I: clap::Args> {
    #[command(flatten)]
    pub id: I,
}
```

`LogsArgs<I>` and `StopArgs<I>` gain the same flattened `id` beside the `--level` and `--force` they already have.

The arg structs take the flags rather than the app, so their `Debug` derives against `F: Debug`, which the associated type satisfies, and no app is asked for a `Debug` it has no use for.

The `Start` and `Daemon` doc comments lose the binary's name, since the derive writes them into every app's `--help`. `LogsArgs` moves across as it is; `RestartArgs` gains the app's flags and keeps `--force`.

`StopArgs` and `RestartArgs` say what `--force` costs, and `Signal`'s two variants say what each signal does. All four of those doc comments are written about mercury today, naming the keyboard grab and the modifiers a command layer swallowed. They are reworded down to what this crate actually knows, which is a process, its pid, a lock, and two signals: SIGTERM asks the daemon to quit and it leaves the way it chose to, SIGKILL destroys it so no destructor runs and whatever it would have undone stays undone. Nothing here knows there is a model, an event, or a keyboard. A doc comment is the one place "mercury" can survive the move without the compiler noticing.

`#[command(name = ...)]` cannot carry `A::NAME`, because a derive macro reads only what is written beside it. The name is set on the built `Command` instead, which is also where `A::ABOUT` goes. Verified on the pinned 1.96.0: `--help` prints `Usage: mercury [COMMAND]` with the app's about line above it.

`use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};`.

## The entry point

`dispatch` takes the matches beside the verb, because the three verbs that put a daemon somewhere have to hand it the flags this invocation was given, and that is read off the matches rather than off the parsed struct. `Typed` is what carries them.

```rust
/// Do what the command line asked, and report the exit code for it.
///
/// Every arm resolves which daemon it means before it does anything, and an id that names none is
/// the one failure this level reports itself.
fn dispatch<A: App>(verb: Option<Verb<A>>, typed: Typed<'_>) -> i32 {
    // The bare invocation is `start` with no id: one behaviour, not two that agree. An app whose
    // `Id` has a required flag has no bare invocation, and clap says so.
    let verb = verb.unwrap_or_else(|| Verb::Start(StartArgs::default_for::<A>()));

    let Some(instance) = A::instance(verb.id()) else {
        return 1;
    };

    match verb {
        Verb::Start(_) => client::start::<A>(&instance, typed),
        Verb::Restart(args) => client::restart::<A>(&instance, &args, typed),
        Verb::Status(_) => client::status::<A>(&instance),
        Verb::Logs(args) => client::logs::<A>(&instance, &args),
        Verb::Stop(args) => client::stop::<A>(&instance, &args),
        Verb::Daemon(args) => daemon::run::<A>(&instance, &args),
    }
}

impl<A: App> Verb<A> {
    /// What this verb said about which daemon it meant. Every verb says something, because every
    /// verb is about one.
    fn id(&self) -> &A::Id {
        match self {
            Self::Start(args) => &args.id,
            Self::Restart(args) => &args.id,
            Self::Status(args) => &args.id,
            Self::Logs(args) => &args.id,
            Self::Stop(args) => &args.id,
            Self::Daemon(args) => &args.id,
        }
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

`start` spawns the daemon and `restart` spawns a replacement. Both have to give it the flags this invocation was given, and neither can know what those flags are: `A::Id` and `A::DaemonArgs` are the app's. The id goes across too, and it is the half that must: a child spawned without it resolves a different instance from the parent that spawned it, takes a different lock, and the `start` that spawned it reports success having started something else.

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
        Self::emit::<A::Id>(matches, &mut argv);
        Self::emit::<A::DaemonArgs>(matches, &mut argv);
        argv
    }

    /// Re-emit one arg set's typed flags onto `argv`.
    fn emit<T: clap::Args>(matches: &ArgMatches, argv: &mut Vec<OsString>) {
        for arg in T::augment_args(Command::new("probe")).get_arguments() {
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
pub(crate) fn run<A: App>(
    instance: &Instance,
    args: &DaemonVerbArgs<A::Id, A::DaemonArgs>,
) -> i32 {
    let log_path = logging::init(A::NAME, instance, &Terminal::Daemon(LogLevel(&args.log_level)));
    info!(path = %log_path.display(), "logging");

    // Before anything that touches the machine. Two of one instance fight over whatever there is
    // only one of, and for mercury that is the keyboard: two of them swallow and re-emit each
    // other's keys forever. The binding must outlive the call (`let _held`, never `let _`):
    // dropping it releases the lock.
    let _held = match freddie_single_instance::acquire(instance.key()) {
        Ok(held) => held,
        Err(e) => {
            error!(daemon = instance.label(), error = %e, "already running; `stop` ends it");
            return 1;
        }
    };

    A::run(&args.id, &args.app)
}
```

The daemon verb reports a code where mercury's exits 0 whatever happened: a lock it could not take and a menu bar it could not create are both a daemon that did not run. An app whose launch agent declines to revive a clean exit depends on this, which mercury's does.

## Logging keyed to the instance

`crates/mercury/src/logging.rs` moves to `freddie_cli` and takes the two names it currently spells as literals: the app's, which is the directory, and the instance's, which is the file. Its `Terminal`/`LogLevel` split, its pid stamp, its file level, and its client layers cross unchanged.

One file per daemon rather than one per app, so `isograph logs --config ./a.json` tails that daemon and nothing else. mercury's path does not move: its instance is `mercury`, so `~/Library/Logs/mercury/mercury.log` is what it was.

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

pub fn init(name: &str, instance: &Instance, terminal: &Terminal<'_>) -> PathBuf {
    let dir = log_dir(name);
    let file_name = format!("{}.log", instance.key());
```

`LOG_FILE` is deleted; `rolling::never(&dir, &file_name)` and `dir.join(file_name)` take its place. The one message `init` writes itself names the daemon rather than the app:

```rust
    for problem in setup {
        warn!("{}: {problem}", instance.label());
    }
```

`key` and `label` are readers on `Instance`, since nothing outside it may build one that has a filename in the wrong half.

## The client verbs

`crates/mercury/src/client.rs` becomes `freddie_cli`'s `client` module, apart from what `install` and `uninstall` need: `Agent`, `KeepAlive`, `label`, `plist_path`, `domain`, `bootout`, `launchctl`, `users_uid`, `NotInstalled`, and the `TRANSIENT` and `ID` constants all stay in mercury. A launch agent says which session type it loads in, when launchd should revive it, and what its label is, and those are answers about one app rather than about daemons. `app-verbs.md` is what mercury reaches those two verbs through.

What does move is every verb that only reads a lock, spawns a binary, signals a pid, or tails a log. No verb changes what it does, what it prints, or what it exits with. Four things change throughout:

- `const APP: &str = "mercury"` is deleted. Five of its use sites become the instance the verb was given; the two under `label` stay in mercury with the launch agent.
- Every function that reads the instance, or calls one that does, takes an `&Instance`, and every one that resolves a name gains `<A: App>`.
- Every message that spells "mercury" spells `instance.label()`, which is what makes both a fork's output its own and isograph's name the config it means.
- `start` and `restart` take a `Typed<'_>` and pass it down to `spawn_daemon`, which forwards the id along with the flags.

The instance reaches the lock, in place of the app's name:

```rust
fn ensure_started<A: App>(instance: &Instance, typed: Typed<'_>) -> Result<Running, NotStarted> {
    match freddie_single_instance::holder(instance.key()) {
```

and every line a verb writes:

```rust
pub(crate) fn status<A: App>(instance: &Instance) -> i32 {
    logging::init(A::NAME, instance, &Terminal::Client);
    match freddie_single_instance::holder(instance.key()) {
        Ok(Held::Free) => {
            info!("{} is not running", instance.label());
            1
        }
        Ok(Held::By(pid)) => {
            info!("{} is running (pid {pid})", instance.label());
            0
        }
```

For mercury the label is `mercury`, so every line reads exactly as it reads today. For isograph it is the config, so two daemons say which of them they are.

The tests move with the module and pick the name up from a test app:

```rust
#[cfg(test)]
struct TestApp;

#[cfg(test)]
impl App for TestApp {
    type Id = NoArgs;
    type DaemonArgs = NoArgs;
    const NAME: &'static str = "testapp";
    const ABOUT: &'static str = "A test app.";

    fn instance(_: &NoArgs) -> Option<Instance> {
        Some(Instance::global(Self::NAME))
    }

    fn run(_: &NoArgs, _: &NoArgs) -> i32 {
        unreachable!("the parse tests never run the daemon")
    }
}
```

`cli.rs`'s parse tests become `Args::<TestApp>::try_parse_from`, and the two that assert a port move to mercury, which is the crate that has one. A second test app with a real `Id` covers what mercury cannot: that two ids resolve to two instances, and that `start` forwards the id into the child.

## What mercury becomes

`crates/mercury/src/main.rs`, entire:

```rust
//! The mercury binary: a freddie app and the command line that runs it.

use freddie_cli::{App, Instance, NoArgs};

mod agent;
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
    // One mercury to a machine, so no flag names which.
    type Id = NoArgs;
    type DaemonArgs = MercuryArgs;
    // `install` and `uninstall`, which are mercury's own. See `app-verbs.md`.
    type Subcommands = agent::MercuryVerb;

    const NAME: &'static str = "mercury";
    const ABOUT: &'static str = "A layered keyboard remapper.";

    fn instance(_: &NoArgs) -> Option<Instance> {
        Some(Instance::global(Self::NAME))
    }

    fn run(_: &NoArgs, args: &MercuryArgs) -> i32 {
        daemon::run(args.port)
    }

    fn run_subcommand(verb: agent::MercuryVerb) -> i32 {
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
2. **`freddie_cli`, holding the command surface, and `app-verbs.md` with it.** The trait, `Instance`, the generic `Args`/`Verb`/arg structs, `main`, `dispatch`, the daemon verb, `logging`, and the `client` module, all keyed to the instance the command line named. mercury shrinks to the `main.rs` above, a `daemon.rs` that starts at `freddie_windows::init()`, and an `agent.rs` holding `install` and `uninstall`. Every verb does what it does now, and `mercury --help` prints what it prints now, because mercury's instance is its name and its `Id` is empty.

`app-verbs.md` lands here rather than after, because `install` and `uninstall` are mercury's own verbs and there is no way to keep them without the seam that carries them.

One change rather than one per verb: `Verb` is a single enum and `dispatch` a single match, so a `freddie_cli` holding some of the verbs would leave mercury without the rest.
