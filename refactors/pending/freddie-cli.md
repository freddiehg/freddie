# the command line is freddie's, the daemon is the app's

An app built here is a process holding something there can only be one of, plus a handful of verbs for finding it, starting it, and stopping it. mercury owns the keyboard, and there is one of it on the machine. isograph v2 watches the directories one config names, and there is one of it per config, so a machine runs as many as you have projects open. What the process is for differs, how many there are differs, and the verbs that manage one do not. All of them live in mercury today, where a second app cannot reach them.

The verbs are the same because none of them looks inside the process. They read a lock file, spawn a binary, send a signal, and tail a log, and every one of those works the same for a program that has nothing to do with keys or events.

`freddie_cli` is a new crate holding the whole command surface. An app supplies its name, what names one of its daemons, its daemon body, and whatever extra flags that body takes; it gets `start`, `restart`, `status`, `logs`, `stop`, and the hidden `daemon` for free, each keyed to the daemon the command line named and writing to that daemon's own log file. mercury becomes an implementation of one trait, plus a command line of its own with those verbs folded into it.

The name of the binary is the app's, not mercury's, and nothing in `freddie_cli` spells "mercury". An app names the block of lifecycle verbs where it flattens them in, and never one of the verbs inside it.

A new crate rather than a dependency, and that is settled rather than assumed: `refactors/past/reuse-existing-crates.md` audited `single-instance`, `service-manager`, and `daemonize` against what these verbs need and none of them fit. `single-instance` cannot probe without acquiring or report a pid, so `status` and `stop` have nothing to build on; `service-manager` cannot express `SuccessfulExit=false`, which is the key the daemon's exit code is tuned to. Whoever forks this gets the lifecycle from here or writes it again themselves.

## What belongs where

`freddie_cli` owns:

- the `Verb` enum and the arg structs under it, for an app to flatten into its own parser
- the single-instance lock: acquiring it for the daemon, probing it for the clients, both under the instance the command line named
- logging setup, and the log directory
- every client verb, since all of them are the lock, the log file, and a subprocess
- delivering SIGTERM to the app as a request to stop

The app owns:

- its name, which is what the log directory is called
- what names one of its daemons, and how many of them there can be
- what the daemon does, which is everything `mercury::daemon` holds today
- any flag beyond `--log-level` that its daemon takes
- what quitting means, since this crate only ever sends a signal and waits for the lock to go free
- whether it installs itself to start with the session, and what that job says

## The seam

```rust
/// What an app is, to the command line that runs it.
///
/// One impl per binary. Everything else in this crate is generic over it, so `TApp` names the app
/// wherever it appears and never one of the app's own types.
pub trait App {
    /// What names one of this app's daemons, as flags every verb takes.
    ///
    /// [`NoArgs`] for an app with one global daemon, which every verb then means without saying
    /// so. An app with more than one says which in a flag, and two values of that flag are two
    /// daemons, each with its own lock, log, and pid.
    type Id: clap::Args + fmt::Debug;

    /// The flags this app's daemon takes beyond the shared ones and beyond [`Id`](Self::Id).
    ///
    /// [`NoArgs`] for an app that takes none. Separate from `Id` so that the verbs which only
    /// find a daemon take only what names one, and refuse the flags that configure one rather
    /// than accepting and ignoring them.
    type DaemonArgs: clap::Args + fmt::Debug;

    /// The name of the binary, which is what the log directory is called.
    ///
    /// Not the help text: the app writes its own `--help`, because the app owns its command line.
    const NAME: &'static str;

    /// Which daemon the command line named.
    ///
    /// Fallible and allowed to touch the filesystem, because naming a daemon can mean reading
    /// one. An id names what the flag points at rather than what it says: two paths to one file
    /// are one daemon, so an app resolves before it keys anything to the result, and a target
    /// that is not there names no daemon at all.
    ///
    /// Returns why rather than saying why. This runs before the log file has a name, since the
    /// name comes from what it returns, so an app that wrote to the log here would write before
    /// there was a subscriber to take it.
    ///
    /// Called before the lock, the log file, and any subprocess, so every verb agrees on which
    /// daemon it meant before it does anything.
    ///
    /// The error is shown once and the process then exits, so a boxed one carries everything
    /// this crate does with it. `freddie_menu_bar::show` returns the same type.
    fn instance(id: &Self::Id) -> Result<Instance, Box<dyn std::error::Error + Send + Sync>>;

    /// Be the daemon. Returns when it has stopped, or when it could not start.
    ///
    /// Called with the lock held and logging initialized. Returning drops the lock, so an app
    /// that wants to stay running stays inside this call.
    ///
    /// Nothing comes back, because there is one exit code a daemon can produce by returning. A
    /// service manager that revives a daemon which exited badly must not revive one that refused
    /// to start, since the next attempt fails the same way, so both of those exit zero. A panic
    /// never reaches here and exits nonzero on its own, which leaves it the only outcome worth
    /// starting the daemon again for.
    fn run(id: &Self::Id, args: &Self::DaemonArgs);
}

/// The daemon flags, or the id, of an app that has none.
#[derive(clap::Args, Debug)]
pub struct NoArgs {}
```

One daemon of one app: what its lock is under, what its log is called, and what a verb calls it when it says something about it.

```rust
/// Which daemon this is, and everything keyed to it.
///
/// The key never leaves: what callers get are the lock and the log path built from it, so the one
/// convention that turns a daemon into two filenames lives here rather than at each place that
/// wants one. A `key` and an app name are all either of them is.
#[derive(Clone, Debug)]
pub struct Instance {
    slug: String,
    display_name: String,
    log_file: PathBuf,
}

impl Instance {
    /// The one daemon of an app that has one, keyed to the app itself.
    pub fn global(app: impl Into<String>) -> Result<Self, NoLogDir> {
        let app = app.into();
        Self::named(&app, app.clone(), app.clone())
    }

    /// One of many: `slug` names its files, `display_name` is what the person who asked for it
    /// typed.
    ///
    /// `slug` has to be stable across two invocations that mean the same daemon, and distinct for
    /// two that do not, since it is the whole of what the lock is keyed to. A path that has been
    /// resolved, or a hash of one, is the shape of it. It also has to be a filename, because it
    /// becomes one.
    ///
    /// Fails when the environment does not say where this user's files go, which is the one thing
    /// about placing a daemon that can fail. It fails here, once, rather than at each call that
    /// wants the path: an instance that exists is one whose files have a place to be.
    pub fn named(
        app: &str,
        slug: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<Self, NoLogDir> {
        let slug = slug.into();
        let log_file = log_dir(app)?.join(format!("{slug}.log"));
        Ok(Self { slug, display_name: display_name.into(), log_file })
    }

    /// What the single-instance lock is keyed to.
    pub fn lock(&self) -> &str {
        &self.slug
    }

    /// Where this daemon's log goes. One directory per app, one file per daemon in it.
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }

    /// What a verb calls this daemon when it says something about it.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

/// The environment names no per-user directory to keep the log in.
///
/// The call this fails on is the one that makes `freddie_single_instance::acquire` return
/// `LockError::NoStateDir`, so a daemon that cannot find its home cannot take its lock either and
/// was never going to run.
#[derive(Debug)]
pub struct NoLogDir;

impl fmt::Display for NoLogDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no per-user directory to keep the log in; is HOME set?")
    }
}

impl std::error::Error for NoLogDir {}

/// The per-user directory this app's logs go in, which is the one platform-shaped thing here.
/// `freddie-cli-off-macos.md` is where it gains its other arms.
fn log_dir(app: &str) -> Result<PathBuf, NoLogDir> {
    let home = std::env::var_os("HOME").ok_or(NoLogDir)?;
    Ok(PathBuf::from(home).join("Library/Logs").join(app))
}
```

An app reaches this through `?`, since `NoLogDir` is an `Error` and `instance` returns a boxed one:

```rust
    fn instance(_: &NoArgs) -> Result<Instance, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Instance::global(Self::NAME)?)
    }
```

Nothing in either type is about freddie. A name, what names one daemon, some flags, and a function that runs until it is done: any program that wants one instance of itself and the verbs to manage it fits, whether or not it has a model, an event, or a keyboard. `freddie-daemon-runtime.md` is what mercury calls inside `run`, and this crate never learns that it exists.

## The verbs, and the flags they carry

An app's daemon takes flags of its own: mercury's `--port` names the socket the extension talks to, and isograph's is a config file path. So the command line is generic over the app, and each of the types that carries the app's flags flattens `TApp::DaemonArgs` in. clap's derive accepts generic parameters, which is what lets it. Verified on the pinned 1.96.0 against clap 4.6.2: a `Subcommand` enum whose variants are a mix of generic and not derives, an arg struct generic over the flags flattens them in, `app daemon --port 4001` parses into it, the shared defaults resolve, and `NoArgs` gives an app with no flags of its own.

Every verb is generic, because every one of them is about one daemon and has to be told which. `status`, `logs`, and `stop` take `TApp::Id` and nothing more, since finding a daemon needs only its name. `daemon`, `start`, and `restart` take `TApp::DaemonArgs` as well, because they are the three that put a daemon somewhere and it needs its flags to run.

For mercury, whose `Id` is `NoArgs`, this adds nothing to any verb: `mercury status` takes no flags today and takes none after.

```rust
/// The lifecycle verbs, for an app to flatten into its own command line.
///
/// Each variant's doc comment is its line in `--help`, so the help text cannot drift from the
/// verbs. Declaration order is help order within this block; where the block sits among the app's
/// own verbs is the app's to choose.
///
/// The derive would put a `Debug` bound on `TApp`, which holds no data here. The types below hold
/// the flags and derive it against those.
#[derive(Subcommand)]
pub enum Verb<TApp: App> {
    /// Start the daemon if it is not running, and exit.
    Start(StartArgs<TApp::Id, TApp::DaemonArgs>),
    /// Stop the running daemon and start a fresh one.
    Restart(RestartArgs<TApp::Id, TApp::DaemonArgs>),
    /// Report whether the daemon is running, and its pid.
    Status(IdArgs<TApp::Id>),
    /// Follow the log, starting nothing.
    Logs(LogsArgs<TApp::Id>),
    /// Ask the running daemon to quit.
    Stop(StopArgs<TApp::Id>),
    /// Run the daemon in this process. Not for typing: `start` spawns it.
    #[command(hide = true)]
    Daemon(DaemonVerbArgs<TApp::Id, TApp::DaemonArgs>),
}

/// What the foreground daemon can be told: what this crate asks of every app, and what the app
/// asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<I: clap::Args, F: clap::Args> {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `warn,some_crate=debug` are both
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

The `Start` and `Daemon` doc comments lose the binary's name, since the derive writes them into every app's `--help`. `LogsArgs` keeps its `--level` and `StopArgs` its `--force`, and both gain the flattened id.

`StopArgs` and `RestartArgs` say what `--force` costs, and `Signal`'s two variants say what each signal does. All four of those doc comments are written about mercury today, naming the keyboard grab and the modifiers a command layer swallowed. They are reworded down to what this crate actually knows, which is a process, its pid, a lock, and two signals: SIGTERM asks the daemon to quit and it leaves the way it chose to, SIGKILL destroys it so no destructor runs and whatever it would have undone stays undone. Nothing here knows there is a model, an event, or a keyboard. A doc comment is the one place "mercury" can survive the move without the compiler noticing.

`use clap::error::ErrorKind; use clap::{ArgMatches, CommandFactory, FromArgMatches, Subcommand};`.

## The app owns its command line

`freddie_cli` hands out an enum rather than taking over `main`. The binary declares its own `Parser`, writes its own name and about line in the derive where clap can see them, lists whatever verbs are its own, and flattens the lifecycle verbs in among them.

```rust
#[derive(Parser)]
#[command(name = "mercury", version, about = "A layered keyboard remapper.", long_about = None)]
struct MercuryCli {
    #[command(subcommand)]
    verb: Option<MercuryVerb>,
}

#[derive(Subcommand)]
enum MercuryVerb {
    /// Everything freddie provides: start, restart, status, logs, stop, and the hidden daemon.
    #[command(flatten)]
    Lifecycle(freddie_cli::Verb<Mercury>),

    /// Register this binary as a login agent, so mercury starts with the session.
    Install,
    /// Take the login agent back out.
    Uninstall,
}
```

The flattened variant sits first because that is where mercury's `--help` lists those verbs today. An app that wants its own first writes them first.

Verified on the pinned 1.96.0 against clap 4.6.2, with mercury's and isograph's command lines built this way: `mercury install` and `mercury status` both parse and reach the right side, `isograph watch ./src` sits beside `isograph status --config ./a.json`, `isograph status --port 1` is still refused, and `mercury --help` prints one flat list of verbs with no sign that half of them came from a library.

## Doing a lifecycle verb

`dispatch` takes the app's whole `ArgMatches` beside the verb, because the three verbs that put a daemon somewhere have to hand it the flags this invocation was given, and those are read off the matches rather than off the parsed struct. `TypedArgs` is what carries them, and it is this crate's own: an app passes what it already has and never builds one.

```rust
/// Do one lifecycle verb, and report the exit code for it.
///
/// Resolves which daemon it means before it does anything, and an id that names none is the one
/// failure this level reports itself.
///
/// Reads the typed flags off the matches itself, so every spawned daemon gets the id its parent
/// was given. One spawned without it resolves a different instance, takes a different lock, and
/// leaves the `start` that spawned it reporting success over something else.
pub fn dispatch<TApp: App>(verb: Verb<TApp>, matches: &ArgMatches) -> i32 {
    let typed = TypedArgs::of(matches);

    let instance = match TApp::instance(verb.id()) {
        Ok(instance) => instance,
        // Through clap rather than a print, and on stderr rather than in the log, because there
        // is no log yet: its path comes from the instance this failed to produce. An id that
        // names no daemon is a bad command line, so it is reported by the thing that reports bad
        // command lines, which `refactors/past/one-log-many-writers.md` already exempts from
        // going through tracing. Verified on the pinned 1.96.0: this writes `error: <what the app
        // said>` to stderr and exits 2, as a rejected flag does.
        Err(e) => clap::Error::raw(ErrorKind::ValueValidation, format!("{e}\n")).exit(),
    };

    match verb {
        Verb::Start(_) => client::start::<TApp>(&instance, typed),
        Verb::Restart(args) => client::restart::<TApp>(&instance, &args, typed),
        Verb::Status(_) => client::status(&instance),
        Verb::Logs(args) => client::logs::<TApp>(&instance, &args),
        Verb::Stop(args) => client::stop::<TApp>(&instance, &args),
        Verb::Daemon(args) => {
            daemon::run::<TApp>(&instance, &args);
            0
        }
    }
}

/// What the bare binary means: `start`, with every flag left unsaid.
///
/// Built by parsing an empty command line, so the app's own defaults decide what it resolves to.
/// An app whose `Id` has a required flag has no bare invocation, and clap says so in the words it
/// uses for a missing flag anywhere else, then exits. Verified on the pinned
/// 1.96.0: an app whose `Id` is [`NoArgs`] resolves its global instance, and one whose `Id` has a
/// required flag exits with `the following required arguments were not provided`.
pub fn bare<TApp: App>() -> Verb<TApp> {
    let command = StartArgs::<TApp::Id, TApp::DaemonArgs>::augment_args(Command::new(TApp::NAME));
    let matches = match command.try_get_matches_from([TApp::NAME]) {
        Ok(matches) => matches,
        Err(e) => e.exit(),
    };
    Verb::Start(
        StartArgs::from_arg_matches(&matches)
            .expect("the derived type matches the command it derived"),
    )
}

impl<TApp: App> Verb<TApp> {
    /// What this verb said about which daemon it meant. Every verb says something, because every
    /// verb is about one.
    fn id(&self) -> &TApp::Id {
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
```

`TypedArgs` is read off the app's own matches, and stays private so that reading it is not something an app does at all:

```rust
impl<'a> TypedArgs<'a> {
    /// The flags the invocation typed, wherever the app's parser put them.
    ///
    /// Taken through `subcommand` so no verb's name is spelled here. `None` is the bare binary,
    /// which typed nothing.
    fn of(matches: &'a ArgMatches) -> Self {
        Self(matches.subcommand().map(|(_name, verb)| verb))
    }
}
```

## Handing the app's flags to the daemon

`start` spawns the daemon and `restart` spawns a replacement. Both have to give it the flags this invocation was given, and neither can know what those flags are: `TApp::Id` and `TApp::DaemonArgs` are the app's. The id goes across too, and it is the half that must: a child spawned without it resolves a different instance from the parent that spawned it, takes a different lock, and the `start` that spawned it reports success having started something else.

clap parses argv into a struct and does not write one back out, so the flags are re-emitted from the matches instead. Reading the same derive that parsed them is what keeps this from drifting: an app that adds a flag gets it forwarded without writing a line.

```rust
/// The app flags this invocation typed, for the process that will run the daemon.
///
/// `None` is the bare binary, which typed nothing. Borrowed from the matches rather than parsed
/// out of them, because what has to be re-emitted is what was written, not what it resolved to.
#[derive(Clone, Copy)]
pub(crate) struct TypedArgs<'a>(Option<&'a ArgMatches>);

impl TypedArgs<'_> {
    /// Re-emit the app's flags as argv for the daemon this process is about to spawn.
    ///
    /// Only what was typed. A default is left out because the daemon resolves the same one, and a
    /// value from the environment is left out because the child inherits the environment it came
    /// from and resolves that too.
    pub(crate) fn argv<TApp: App>(self) -> Vec<OsString> {
        let Some(matches) = self.0 else {
            return Vec::new();
        };
        let mut argv = Vec::new();
        Self::emit::<TApp::Id>(matches, &mut argv);
        Self::emit::<TApp::DaemonArgs>(matches, &mut argv);
        argv
    }

    /// Re-emit one arg set's typed flags onto `argv`.
    fn emit<T: clap::Args>(matches: &ArgMatches, argv: &mut Vec<OsString>) {
        for arg in T::augment_args(Command::new("probe")).get_arguments() {
            let id = arg.get_id().as_str();
            let carried = matches.value_source(id) == Some(ValueSource::CommandLine);
            // A positional has no flag to re-emit it under. An app's two arg sets are sets of
            // flags: the verbs own their positionals, and an app's would be ambiguous against
            // them.
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

Verified on the pinned 1.96.0 against clap 4.6.2, against a `DaemonArgs` of `--port` (with an env and a default), `--config` (an optional path), and `--verbose` (a flag):

- `start --port 4001 --config "/a b/c.toml"` re-emits exactly `["--port", "4001", "--config", "/a b/c.toml"]`. The value with a space needs no quoting, because these reach `Command::arg` as separate `OsString`s and no shell parses them.
- `restart --force --verbose` re-emits `["--verbose"]`. The verb's own `--force` drops out with no stripping, because it is not one of the app's arguments.
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
fn spawn_daemon<TApp: App>(typed: TypedArgs<'_>) -> io::Result<Pid> {
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("daemon")
        .args(typed.argv::<TApp>())
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
/// Run the app's daemon in the foreground: set up logging, take the lock, and hand over.
pub(crate) fn run<TApp: App>(
    instance: &Instance,
    args: &DaemonVerbArgs<TApp::Id, TApp::DaemonArgs>,
) {
    let log_path = logging::init(instance, &Terminal::Daemon(LogLevel(&args.log_level)));
    info!(path = %log_path.display(), "logging");

    // Before anything that touches the machine, because two of one instance fight over whatever
    // there is only one of. The binding must outlive the call (`let _held`, never `let _`):
    // dropping it releases the lock.
    let _held = match freddie_single_instance::acquire(instance.lock()) {
        Ok(held) => held,
        Err(e) => {
            error!(daemon = instance.display_name(), error = %e, "already running; `stop` ends it");
            return;
        }
    };

    TApp::run(&args.id, &args.app);
}
```

A second daemon that finds the lock held has not failed, it has found out it was not needed, and it leaves the way a clean stop does.

## Logging keyed to the instance

`crates/mercury/src/logging.rs` moves to `freddie_cli` and stops deciding where the log goes: the instance knows. Its `Terminal`/`LogLevel` split, its pid stamp, its file level, and its client layers cross unchanged.

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
pub fn init(instance: &Instance, terminal: &Terminal<'_>) -> PathBuf {
    let path = instance.log_file();
    let dir = path.parent().unwrap_or(Path::new("."));
    let file_name = path.file_name().unwrap_or(OsStr::new("log"));
```

`LOG_FILE` and this module's `log_dir` are both deleted; the instance was built knowing its path, and `rolling::never(dir, file_name)` splits it because that is the shape `tracing_appender` takes. The `unwrap_or`s are unreachable for a path built by `Instance::named`, and are there because `Path` cannot say so. The one message `init` writes itself names the daemon rather than the app:

```rust
    for problem in setup {
        warn!("{}: {problem}", instance.display_name());
    }
```

## The client verbs

`crates/mercury/src/client.rs` becomes `freddie_cli`'s `client` module, apart from what `install` and `uninstall` need: `Agent`, `KeepAlive`, `label`, `plist_path`, `domain`, `bootout`, `launchctl`, `users_uid`, `NotInstalled`, and the `TRANSIENT` and `ID` constants all stay in mercury, in `agent.rs`. A launch agent says which session type it loads in, when launchd should revive it, and what its label is, and those are answers about one app rather than about daemons. mercury reaches them through its own two verbs, which is what owning its command line is for. The label keeps its `hg.freddie.` prefix, mercury's to choose now that no shared crate spells it.

What does move is every verb that only reads a lock, spawns a binary, signals a pid, or tails a log. No verb changes what it does, what it prints, or what it exits with. Four things change throughout:

- `const APP: &str = "mercury"` is deleted. Five of its use sites become the instance the verb was given; the two under `label` stay in mercury with the launch agent.
- Every function that reads the instance, or calls one that does, takes an `&Instance`, and every one that resolves a name gains `<TApp: App>`.
- Every message that spells "mercury" spells `instance.display_name()`, which is what makes both a fork's output its own and isograph's name the config it means.
- `start` and `restart` take a `TypedArgs<'_>` and pass it down to `spawn_daemon`, which forwards the id along with the flags.

The instance reaches the lock, in place of the app's name:

```rust
fn ensure_started<TApp: App>(instance: &Instance, typed: TypedArgs<'_>) -> Result<Running, NotStarted> {
    match freddie_single_instance::holder(instance.lock()) {
```

and every line a verb writes:

```rust
pub(crate) fn status(instance: &Instance) -> i32 {
    logging::init(instance, &Terminal::Client);
    match freddie_single_instance::holder(instance.lock()) {
        Ok(Held::Free) => {
            info!("{} is not running", instance.display_name());
            1
        }
        Ok(Held::By(pid)) => {
            info!("{} is running (pid {pid})", instance.display_name());
            0
        }
```

For mercury the display name is `mercury`, so every line reads exactly as it reads today. For isograph it is the config, so two daemons say which of them they are.

The tests move with the module and pick the name up from a test app:

```rust
#[cfg(test)]
struct TestApp;

#[cfg(test)]
impl App for TestApp {
    type Id = NoArgs;
    type DaemonArgs = NoArgs;
    const NAME: &'static str = "testapp";

    fn instance(_: &NoArgs) -> Result<Instance, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Instance::global(Self::NAME)?)
    }

    fn run(_: &NoArgs, _: &NoArgs) {
        unreachable!("the parse tests never run the daemon")
    }
}
```

`cli.rs`'s parse tests move to mercury with its command line, since that is the crate that has a `Parser` now; what stays here are tests over `Verb<TestApp>` flattened into a test `Parser`, and the two that assert a port go to mercury. A second test app with a real `Id` covers what mercury cannot: that two ids resolve to two instances, and that `start` forwards the id into the child.

## What mercury becomes

`crates/mercury/src/main.rs`, entire:

```rust
//! The mercury binary: its command line, with freddie's lifecycle verbs folded into it.

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use freddie_cli::{App, Instance, NoArgs};

mod agent;
mod daemon;

#[derive(Parser)]
#[command(name = "mercury", version, about = "A layered keyboard remapper.", long_about = None)]
struct MercuryCli {
    #[command(subcommand)]
    verb: Option<MercuryVerb>,
}

#[derive(Subcommand)]
enum MercuryVerb {
    /// start, restart, status, logs, stop, and the hidden daemon.
    #[command(flatten)]
    Lifecycle(freddie_cli::Verb<Mercury>),

    /// Register this binary as a login agent, so mercury starts with the session.
    Install,
    /// Take the login agent back out.
    Uninstall,
}

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

    const NAME: &'static str = "mercury";

    fn instance(_: &NoArgs) -> Result<Instance, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Instance::global(Self::NAME)?)
    }

    fn run(_: &NoArgs, args: &MercuryArgs) {
        daemon::run(args.port);
    }
}

fn main() -> ! {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    // The matches are kept beside the parse because `dispatch` reads what was written from them.
    let matches = MercuryCli::command().get_matches();
    let cli = MercuryCli::from_arg_matches(&matches)
        .expect("the derived type matches the command it derived");

    let code = match cli.verb {
        Some(MercuryVerb::Lifecycle(verb)) => freddie_cli::dispatch::<Mercury>(verb, &matches),
        Some(MercuryVerb::Install) => agent::install(),
        Some(MercuryVerb::Uninstall) => agent::uninstall(),
        None => freddie_cli::dispatch::<Mercury>(freddie_cli::bare::<Mercury>(), &matches),
    };
    // Every path above has returned, so the daemon's locals are already dropped by the time this
    // skips the rest of the destructors.
    std::process::exit(code);
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
pub(crate) fn run(port: u16) {
    if let Err(e) = freddie_windows::init() {
```

Its signature is what it is today, and its two early returns stay early returns. `use crate::cli::DaemonArgs;` and `use crate::logging::{self, LogLevel, Terminal};` go with the modules they name.

`freddie_cli`'s dependencies are `clap`, `tracing`, `tracing-subscriber`, `tracing-appender`, and `freddie_single_instance`. mercury drops `tracing-subscriber` and `tracing-appender`, and keeps `clap`, `tracing`, `serde` for the wire types and the launch agent, `plist` for writing it, and `freddie_single_instance`, which `label` and the agent verbs still reach.

## The changes, in order

1. **Logging takes a name.** `log_dir(name)`, the file name derived from it, and `init(name, terminal)`, in mercury, with `client::APP` passed at each of the four call sites. No behaviour changes: the name passed is the name that was written. The instance is not part of this one, since mercury has no second daemon to tell apart; change 2 is where `init` gains it.
2. **`freddie_cli`, holding the lifecycle verbs.** The trait, `Instance`, `Verb` and the arg structs, `dispatch`, `bare`, the daemon verb, `logging`, and the `client` module, all keyed to the instance the command line named. mercury gains a `Parser` of its own that flattens `Verb<Mercury>` in beside `install` and `uninstall`. mercury shrinks to the `main.rs` above, a `daemon.rs` that starts at `freddie_windows::init()`, and an `agent.rs` holding `install` and `uninstall`. Every verb does what it does now, and `mercury --help` prints what it prints now, because mercury's instance is its name and its `Id` is empty.

One change rather than one per verb: `Verb` is a single enum and `dispatch` a single match, so a `freddie_cli` holding some of the verbs would leave mercury without the rest.
