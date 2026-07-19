//! The mercury command line.
//!
//! One process runs the model and owns the keyboard (`mercury daemon`, in `daemon.rs`); every
//! other verb is a client of it.

use clap::Parser;

use cli::{Args, DaemonArgs, Verb};

mod cli;
mod daemon;
mod logging;

/// Do what the command line asked, and report the exit code for it.
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        // The bare `mercury`. `mercury-start.md` makes this start the daemon and follow its log;
        // until then it is what running mercury has always been.
        None => {
            daemon::run(&DaemonArgs::default());
            0
        }
        Some(Verb::Daemon(args)) => {
            daemon::run(&args);
            0
        }
    }
}

fn main() {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    // clap exits the process itself rather than handing back an error, so there is no error arm
    // here; the tests reach the parser through `try_parse_from`, which returns instead.
    let code = run(Args::parse().verb);
    // Every path above has returned, so the daemon's locals (the lock, the menu bar, the run loop
    // stopper) are already dropped by the time this skips the rest of the destructors.
    std::process::exit(code);
}
