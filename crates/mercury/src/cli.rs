//! What mercury can be told at startup.

use clap::Parser;

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
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}
