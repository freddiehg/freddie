# the daemon is a verb

mercury already is the daemon. It runs one process, owns the keyboard, and drives the model, and none of that changes. What changes is that being that process becomes one thing the binary can be told to do, rather than the only thing: `mercury daemon` runs it in the foreground, and `main` becomes a parse and a dispatch.

Nothing a user sees changes. `mercury` with no verb still runs the daemon exactly as it does today, and the verbs that make it a background process arrive in `mercury-stop.md` and `mercury-start.md`. This is the prefactor those need, and the point at which the daemon's code stops living in `main.rs`.

`launch-at-login.md` gets its plist command out of this: `ProgramArguments` becomes `[/usr/local/bin/mercury, daemon]`, naming the foreground run explicitly rather than relying on the bare binary's default, which later changes.

## The files

- `main.rs` parses and dispatches, and holds nothing else.
- `cli.rs` holds the clap types, which is what its module doc already says it is for.
- `daemon.rs` is new, and is everything `main.rs` holds today apart from `fn main`.
- `client.rs` arrives with the first client verb, in `mercury-status-and-logs.md`.

Everything in `main.rs` today except `fn main` moves verbatim into `daemon.rs`: `run`, `run_event_loop`, `dispatch_event`, `run_effect_loop`, `perform_effect`, `schedule_timer`, `place_window`, `foreground_app`, the imports they need, and the module doc describing the threads, which describes the daemon and belongs with it.

Two renames, because the file gains a second entry point and cannot have two `run`s:

- today's `fn main` body becomes `pub(crate) fn run(args: &DaemonArgs)`
- today's `async fn run` becomes `async fn serve`, with its `run(event_tx, event_rx, title_tx, args.port)` call site becoming `serve(event_tx, event_rx, title_tx, port)`, reading `let port = args.port;` before the worker's `move` closure takes it

`run` takes the whole `DaemonArgs` rather than a flag at a time, so a later flag is a field rather than another positional argument through `main`.

`mod logging;` stays declared in `main.rs`, and `daemon.rs` reaches it as `crate::logging`.

## `--log-level` and `--port` move onto the verb

`--log-level` governs the terminal subscriber and nothing else, the log file recording `debug` whatever it says. A daemon started in the background has no terminal, so the flag has nothing to do there. `--port` names the socket the running daemon listens on. Both configure a daemon, so both belong on the verb rather than on the bare binary.

`cli.rs`, before:

```rust
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
```

After:

```rust
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
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs),
}

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
```

`use clap::{Parser, Subcommand};` at the top, and the `Args` doc comment's reference to `main` stays accurate.

`LOG_LEVEL` and `MERCURY_PORT` still work, so `LOG_LEVEL=mercury=debug mercury daemon` reaches the same place `--log-level` does. What stops working is `mercury --log-level debug`, which clap now refuses as an unknown top-level flag: the value goes after the verb.

Verified on the pinned 1.96.0 against clap 4.6.2: this derives clean under the workspace's `deny` on clippy `all`, `pedantic`, `nursery`, and `cargo`, `mercury daemon` defaults to `info`, `LOG_LEVEL=warn mercury daemon` reads `warn`, and a top-level `--log-level` exits 2.

## `main.rs`

```rust
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
```

The bare `mercury` parses no `DaemonArgs`, so `cli.rs` builds the one it runs on:

```rust
/// What the terminal shows when nothing says otherwise. Shared by [`DaemonArgs`]'s clap default
/// and its [`Default`], which is what the bare `mercury` runs on.
pub const DEFAULT_LOG_LEVEL: &str = "info";

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
```

Two sets of defaults, one written by clap and one by hand, so a test pins them together.

## Tests

In `cli.rs`, since that is where the types are.

```rust
#[cfg(test)]
mod tests {
    use super::{Args, DaemonArgs, Verb};
    use clap::Parser;

    fn parse(args: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("mercury").chain(args.iter().copied()))
            .expect("a valid command line")
    }

    #[test]
    fn no_verb_runs_the_daemon() {
        assert!(parse(&[]).verb.is_none());
    }

    fn daemon_args(args: &[&str]) -> DaemonArgs {
        let Some(Verb::Daemon(args)) = parse(args).verb else {
            panic!("the daemon verb parses to Verb::Daemon");
        };
        args
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

    #[test]
    fn an_unknown_verb_is_refused() {
        assert!(Args::try_parse_from(["mercury", "frobnicate"]).is_err());
    }
}
```

Each later doc extends these: a `Verb` variant carrying its help line, its `run` arm, and a test asserting it parses.

## Verifying

`cargo run -p mercury` and `cargo run -p mercury -- daemon` behave identically and identically to today: the icon appears, the keyboard remaps, `q` from home quits. `cargo run -p mercury -- daemon --log-level mercury=debug` puts debug lines on the terminal, and `mercury --help` lists the one verb.
