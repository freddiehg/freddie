//! What mercury can be told at startup.

use clap::{Parser, Subcommand};
use tracing::Level;

/// Everything mercury can be told at startup.
///
/// Each field is a flag, an environment variable, and a default, in that order of precedence,
/// which clap resolves. A flag it does not recognize, or a value that does not parse, exits with a
/// message naming the offender before [`main`](crate::main) runs a line of its own.
/// `long_about = None` keeps the doc comment above out of `--help`: it is written for whoever
/// reads this file, and clap would otherwise print it verbatim to whoever runs the binary.
#[derive(Parser, Debug)]
#[command(name = "mercury", version, about = "A layered keyboard remapper.", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub verb: Option<Verb>,
}

/// What the command line asked mercury to do, where `None` is the bare `mercury`.
///
/// Each variant's doc comment is its line in `mercury --help`, so the help text cannot drift from
/// the verbs the way a hand-maintained usage string does. Declaration order is help order.
#[derive(Subcommand, Debug)]
pub enum Verb {
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Follow the log, starting nothing.
    Logs(LogsArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
}

/// What `mercury logs` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct LogsArgs {
    /// The least severe records to show: `error`, `warn`, `info`, `debug`, or `trace`.
    ///
    /// The file always records `debug`, whatever this says, so this widens or narrows what reaches
    /// the terminal and never what is kept. Defaults to what a daemon's own terminal defaults to.
    #[arg(long, default_value = DEFAULT_LOG_LEVEL)]
    pub level: Level,
}

/// What `mercury stop` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct StopArgs {
    /// Destroy the daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so a modifier the command
    /// layer swallowed is left down in whatever app was in front.
    #[arg(long)]
    pub force: bool,
}

/// What the terminal shows when nothing says otherwise. Shared by [`DaemonArgs`]'s clap default
/// and its [`Default`], which is what the bare `mercury` runs on.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// What the foreground daemon can be told.
///
/// Its own struct rather than fields on the variant, because a variant carries a struct or carries
/// nothing.
///
/// Every flag here configures the running daemon, so every one of them is on this verb rather than
/// on `mercury` itself.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct DaemonArgs {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on.
    #[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]
    pub log_level: String,

    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}

/// What the bare `mercury` runs on, having parsed no `DaemonArgs` to carry anything.
///
/// `the_bare_mercury_matches_the_daemon_verb` asserts this against what clap produces for
/// `mercury daemon`, so the two spellings cannot drift into meaning different things.
impl Default for DaemonArgs {
    fn default() -> Self {
        Self {
            log_level: DEFAULT_LOG_LEVEL.to_owned(),
            port: mercury::DEFAULT_PORT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, DaemonArgs, Verb};
    use clap::Parser;
    use tracing::Level;

    fn parse(args: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("mercury").chain(args.iter().copied()))
            .expect("a valid command line")
    }

    fn daemon_args(args: &[&str]) -> DaemonArgs {
        let Some(Verb::Daemon(args)) = parse(args).verb else {
            panic!("the daemon verb parses to Verb::Daemon");
        };
        args
    }

    #[test]
    fn no_verb_runs_the_daemon() {
        assert!(parse(&[]).verb.is_none());
    }

    // The bare `mercury` builds its `DaemonArgs` by hand rather than through clap, so nothing but
    // this test keeps the two sets of defaults in step.
    #[test]
    fn the_bare_mercury_matches_the_daemon_verb() {
        assert_eq!(daemon_args(&["daemon"]), DaemonArgs::default());
    }

    #[test]
    fn the_daemon_verb_defaults_to_info() {
        assert_eq!(daemon_args(&["daemon"]).log_level, super::DEFAULT_LOG_LEVEL);
    }

    #[test]
    fn the_daemon_verb_takes_a_filter_directive() {
        assert_eq!(
            daemon_args(&["daemon", "--log-level", "mercury=debug"]).log_level,
            "mercury=debug"
        );
    }

    #[test]
    fn the_daemon_verb_takes_a_port() {
        assert_eq!(daemon_args(&["daemon", "--port", "4001"]).port, 4001);
    }

    #[test]
    fn the_log_level_is_not_a_top_level_flag() {
        assert!(Args::try_parse_from(["mercury", "--log-level", "debug"]).is_err());
    }

    #[test]
    fn the_port_is_not_a_top_level_flag() {
        assert!(Args::try_parse_from(["mercury", "--port", "4001"]).is_err());
    }

    fn stop_args(args: &[&str]) -> super::StopArgs {
        let Some(Verb::Stop(args)) = parse(args).verb else {
            panic!("the stop verb parses to Verb::Stop");
        };
        args
    }

    #[test]
    fn stop_is_gentle_by_default() {
        assert!(!stop_args(&["stop"]).force);
    }

    #[test]
    fn stop_takes_force() {
        assert!(stop_args(&["stop", "--force"]).force);
    }

    #[test]
    fn the_read_only_verbs_parse() {
        assert!(matches!(parse(&["status"]).verb, Some(Verb::Status)));
        assert!(matches!(parse(&["logs"]).verb, Some(Verb::Logs(_))));
    }

    fn logs_args(args: &[&str]) -> super::LogsArgs {
        let Some(Verb::Logs(args)) = parse(args).verb else {
            panic!("the logs verb parses to Verb::Logs");
        };
        args
    }

    // The daemon's terminal and the log follower show the same records unless told otherwise, so
    // they default to one constant rather than to two that happen to match.
    #[test]
    fn logs_defaults_to_the_daemon_default() {
        assert_eq!(
            logs_args(&["logs"]).level.to_string().to_lowercase(),
            super::DEFAULT_LOG_LEVEL
        );
    }

    #[test]
    fn logs_takes_a_level() {
        assert_eq!(logs_args(&["logs", "--level", "debug"]).level, Level::DEBUG);
    }

    #[test]
    fn an_unknown_verb_is_refused() {
        assert!(Args::try_parse_from(["mercury", "frobnicate"]).is_err());
    }
}
