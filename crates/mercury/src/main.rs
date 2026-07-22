//! The mercury binary: its command line, with freddie's lifecycle verbs folded into it.

use std::process::ExitCode;

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
    ///
    /// Flattened first, so `--help` lists them where mercury has always listed them.
    #[command(flatten)]
    Lifecycle(freddie_cli::Verb<Mercury>),

    /// mercury's own: the launch agent.
    #[command(flatten)]
    Agent(agent::MercuryVerb),
}

/// The loopback port the event socket listens on, and nothing else mercury needs from a flag.
#[derive(clap::Args, Debug)]
pub struct MercuryArgs {
    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}

/// mercury, to the verbs that manage it.
pub struct Mercury;

impl App for Mercury {
    // One mercury to a machine, so no flag names which.
    type Id = NoArgs;
    type DaemonArgs = MercuryArgs;

    const NAME: &'static str = "mercury";

    fn instance(_: &NoArgs) -> Result<Instance, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Instance::global(Self::NAME)?)
    }

    fn run_daemon(_: &NoArgs, args: &MercuryArgs) {
        daemon::run(args.port);
    }
}

fn main() -> ExitCode {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    // The matches are kept beside the parse because `run_lifecycle_verb` reads what was written
    // from them, to forward to the daemon it spawns.
    let matches = MercuryCli::command().get_matches();
    let cli = MercuryCli::from_arg_matches(&matches)
        .expect("the derived type matches the command it derived");

    match cli.verb {
        Some(MercuryVerb::Lifecycle(verb)) => {
            freddie_cli::run_lifecycle_verb::<Mercury>(verb, &matches)
        }
        Some(MercuryVerb::Agent(agent::MercuryVerb::Install)) => agent::install(),
        Some(MercuryVerb::Agent(agent::MercuryVerb::Uninstall)) => agent::uninstall(),
        None => freddie_cli::run_lifecycle_verb::<Mercury>(
            freddie_cli::verb_for_bare_invocation::<Mercury>(),
            &matches,
        ),
    }
}
