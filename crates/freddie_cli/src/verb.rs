//! The lifecycle verbs, and what each of them can be told.

use crate::App;
use crate::logging::Terminal;

/// The lifecycle verbs, for an app to flatten into its own command line.
///
/// Each variant's doc comment is its line in `--help`, so the help text cannot drift from the
/// verbs. Declaration order is help order within this block; where the block sits among the app's
/// own verbs is the app's to choose.
///
/// The derive would put a `Debug` bound on `TApp`, which holds no data here. The types below hold
/// the flags and derive it against those.
#[derive(clap::Subcommand)]
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

impl<TApp: App> Verb<TApp> {
    /// What this verb's terminal is for: being the daemon, or talking to one.
    pub(crate) const fn terminal(&self) -> Terminal {
        match self {
            Self::Daemon(_) => Terminal::Daemon,
            Self::Start(_) | Self::Restart(_) | Self::Status(_) | Self::Logs(_) | Self::Stop(_) => {
                Terminal::Client
            }
        }
    }

    /// What this verb said about which daemon it meant. Every verb says something, because every
    /// verb is about one.
    pub(crate) const fn id(&self) -> &TApp::Id {
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

/// What the foreground daemon can be told: which daemon to be, and what the app asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonVerbArgs<I: clap::Args, F: clap::Args> {
    #[command(flatten)]
    pub id: I,

    #[command(flatten)]
    pub app: F,
}

/// What `start` can be told, which is whatever the daemon it spawns can be told.
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

/// What `logs` can be told.
#[derive(clap::Args, Debug)]
pub struct LogsArgs<I: clap::Args> {
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    ///
    /// The file always records `debug`, whatever this says, so this widens or narrows what
    /// reaches the terminal and never what is kept.
    #[arg(long, default_value = crate::logging::DEFAULT_LOG_LEVEL)]
    pub level: tracing::Level,

    #[command(flatten)]
    pub id: I,
}

/// What `stop` can be told.
#[derive(clap::Args, Debug)]
pub struct StopArgs<I: clap::Args> {
    /// Destroy the daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so whatever the app undoes on
    /// the way out is left undone.
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub id: I,
}
