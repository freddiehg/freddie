//! The mercury command line.
//!
//! One process runs the model and owns the keyboard (`mercury daemon`, in `daemon.rs`); every
//! other verb is a client of it.

use clap::Parser;

use cli::{Args, Verb};

mod cli;
mod client;
mod daemon;
mod logging;

/// Do what the command line asked, and report the exit code for it.
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        // The bare `mercury` is `mercury start`: one behaviour, not two that agree.
        None | Some(Verb::Start) => client::start(),
        Some(Verb::Restart(args)) => client::restart(&args),
        Some(Verb::Status) => client::status(),
        Some(Verb::Logs(args)) => client::logs(&args),
        Some(Verb::Stop(args)) => client::stop(&args),
        Some(Verb::Install) => client::install(),
        Some(Verb::Uninstall) => client::uninstall(),
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
