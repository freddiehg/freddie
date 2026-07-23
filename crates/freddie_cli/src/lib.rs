//! The verbs that manage one daemon: finding it, starting it, stopping it, and following its log.
//!
//! A program built on this holds something there can only be one of, and gets `start`, `restart`,
//! `status`, `logs`, `stop`, and a hidden `daemon` for free, each keyed to the daemon the command
//! line named. Nothing here looks inside the process: the verbs read a lock file, spawn a binary,
//! signal a pid, and tail a log, and every one of those works the same whatever the daemon is for.
//!
//! An app declares its own [`clap::Parser`] and flattens [`Verb`] into it, so the binary's name,
//! its about line, and any verbs of its own are the app's to write.

use std::ffi::OsString;
use std::fmt;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{ArgMatches, Args as _, Command, FromArgMatches as _};

mod client;
mod daemon;
mod instance;
mod logging;
mod verb;

pub use instance::{Instance, NoUserDir};
pub use logging::LOG_LEVEL;
pub use verb::{DaemonVerbArgs, IdArgs, LogsArgs, RestartArgs, StartArgs, StopArgs, Verb};

/// What the daemon verb is called, for an app building an argv that has to reach it.
pub const DAEMON_VERB: &str = "daemon";

/// What an app is, to the verbs that manage its daemon.
///
/// One impl per binary. Everything else in this crate is generic over it, so `TApp` names the app
/// wherever it appears and never one of the app's own types.
pub trait App {
    /// What names one of this app's daemons, as flags every verb takes.
    ///
    /// [`NoArgs`] for an app with one global daemon, which every verb then means without saying
    /// so. An app with more than one says which in a flag, and two values of that flag are two
    /// daemons, each with its own lock, log, and pid.
    ///
    /// A field [`instance`](Self::instance) does not read still reaches the daemon, since the id
    /// is handed to [`run_daemon`](Self::run_daemon) whole and forwarded to a spawned one. What
    /// it does not do is tell two daemons apart: `status`, `logs`, and `stop` parse it and then
    /// use nothing but the instance, and two `start`s differing only in such a field are one
    /// daemon, where the second is told one is already running and its value never takes effect.
    type Id: clap::Args + fmt::Debug;

    /// The flags this app's daemon takes beyond [`Id`](Self::Id).
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
    /// # Errors
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
    fn run_daemon(id: &Self::Id, args: &Self::DaemonArgs);
}

/// The daemon flags, or the id, of an app that has none.
#[derive(clap::Args, Debug)]
pub struct NoArgs;

/// Send this process's tracing to `instance`'s log file and to a client verb's terminal.
///
/// [`run_lifecycle_verb`] does this for the verbs it runs. An app's own verbs are not among them,
/// so one that wants its output on a terminal and in the log calls this first, naming the daemon
/// whose log it belongs in.
pub fn init_client_logging(instance: &Instance) {
    logging::init(instance, logging::Terminal::Client);
}

/// Work out which daemon a lifecycle verb means, start logging for it, and run the verb.
///
/// The three steps every verb needs, in the one order they work in. Logging is set up here rather
/// than inside each verb because its path comes from the instance, so nothing before this line
/// has anywhere to write.
///
/// Takes the whole `ArgMatches` because the verbs that spawn a daemon hand it the flags this
/// invocation was given, and those are read off the matches rather than off the parsed struct.
#[must_use]
pub fn run_lifecycle_verb<TApp: App>(verb: Verb<TApp>, matches: &ArgMatches) -> ExitCode {
    let instance = match TApp::instance(verb.id()) {
        Ok(instance) => instance,
        // Through clap rather than a print, and on stderr rather than in the log, because there
        // is no log yet: its path comes from the instance this failed to produce. An id that
        // names no daemon is a bad command line, so it is reported by the thing that reports bad
        // command lines, which `refactors/past/one-log-many-writers.md` exempts from tracing.
        Err(e) => clap::Error::raw(ErrorKind::ValueValidation, format!("{e}\n")).exit(),
    };

    logging::init(&instance, verb.terminal());
    run_verb_on::<TApp>(verb, &instance, TypedArgs::of(matches))
}

/// Run the verb against the daemon it named, with logging already up.
fn run_verb_on<TApp: App>(verb: Verb<TApp>, instance: &Instance, typed: TypedArgs<'_>) -> ExitCode {
    match verb {
        Verb::Start(_) => client::start::<TApp>(instance, typed),
        Verb::Restart(args) => client::restart::<TApp>(instance, args.force, typed),
        Verb::Status(_) => client::status(instance),
        Verb::Logs(args) => client::logs(
            instance,
            client::LogsView {
                least: args.level,
                include_state: args.include_state,
                json: args.json,
            },
        ),
        Verb::Stop(args) => client::stop(instance, args.force),
        Verb::Daemon(args) => {
            daemon::run_in_foreground::<TApp>(instance, &args);
            ExitCode::SUCCESS
        }
    }
}

/// What the bare binary means: `start`, with every flag left unsaid.
///
/// Built by parsing an empty command line, so the app's own defaults decide what it resolves to.
/// An app whose `Id` has a required flag has no bare invocation, and clap says so in the words it
/// uses for a missing flag anywhere else, then exits.
///
/// An app calls this for its `None` arm, where its parser saw no verb at all.
///
/// # Panics
///
/// Panics if the derived parser and the derived command disagree, which is a bug in this crate
/// rather than anything a caller can cause.
#[must_use]
pub fn verb_for_bare_invocation<TApp: App>() -> Verb<TApp> {
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

/// The app flags this invocation typed, for the process that will run the daemon.
///
/// `None` is the bare binary, which typed nothing. Borrowed from the matches rather than parsed
/// out of them, because what has to be re-emitted is what was written, not what it resolved to.
#[derive(Clone, Copy)]
pub(crate) struct TypedArgs<'a>(Option<&'a ArgMatches>);

impl<'a> TypedArgs<'a> {
    /// The flags the invocation typed, wherever the app's parser put them.
    ///
    /// Taken through `subcommand` so no verb's name is spelled here.
    fn of(matches: &'a ArgMatches) -> Self {
        Self(matches.subcommand().map(|(_name, verb)| verb))
    }
}

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
        use clap::builder::ArgAction;
        use clap::parser::ValueSource;

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
