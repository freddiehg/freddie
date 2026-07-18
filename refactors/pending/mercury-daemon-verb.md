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

- today's `fn main` body becomes `pub(crate) fn run(log_level: &str)`
- today's `async fn run` becomes `async fn serve`, with its `run(event_tx, event_rx, title_tx)` call site becoming `serve(...)`

`mod logging;` stays declared in `main.rs`, and `daemon.rs` reaches it as `crate::logging`.

## `--log-level` moves onto the verb

The flag governs the terminal subscriber and nothing else, the log file recording `debug` whatever it says. A daemon started in the background has no terminal, so the flag has nothing to do there. It belongs on the verb where it works.

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
#[derive(clap::Args, Debug)]
pub struct DaemonArgs {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on, which is why this
    /// sits on this verb rather than on `mercury` itself.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,
}
```

`use clap::{Parser, Subcommand};` at the top, and the `Args` doc comment's reference to `main` stays accurate.

`LOG_LEVEL` still works, so `LOG_LEVEL=mercury=debug mercury daemon` reaches the same place `--log-level` does. What stops working is `mercury --log-level debug`, which clap now refuses as an unknown top-level flag: the value goes after the verb.

Verified on the pinned 1.96.0 against clap 4.6.2: this derives clean under the workspace's `deny` on clippy `all`, `pedantic`, `nursery`, and `cargo`, `mercury daemon` defaults to `info`, `LOG_LEVEL=warn mercury daemon` reads `warn`, and a top-level `--log-level` exits 2.

## `main.rs`

```rust
//! The mercury command line.
//!
//! One process runs the model and owns the keyboard (`mercury daemon`, in `daemon.rs`); every
//! other verb is a client of it.

use clap::Parser;

use cli::{Args, Verb};

mod cli;
mod daemon;
mod logging;

/// Do what the command line asked, and report the exit code for it.
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        // The bare `mercury`. `mercury-start.md` makes this start the daemon and follow its log;
        // until then it is what running mercury has always been.
        None => {
            daemon::run(DEFAULT_LOG_LEVEL);
            0
        }
        Some(Verb::Daemon(args)) => {
            daemon::run(&args.log_level);
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

The bare `mercury` has no `DaemonArgs` to read a level from, so `cli.rs` names the one both spellings default to:

```rust
/// What the terminal shows when nothing says otherwise. Shared by [`DaemonArgs`]'s default and by
/// the bare `mercury`, which parses no `DaemonArgs` to carry one.
pub const DEFAULT_LOG_LEVEL: &str = "info";
```

with `#[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]`.

## Tests

In `cli.rs`, since that is where the types are.

```rust
#[cfg(test)]
mod tests {
    use super::{Args, Verb};
    use clap::Parser;

    fn parse(args: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("mercury").chain(args.iter().copied()))
            .expect("a valid command line")
    }

    #[test]
    fn no_verb_runs_the_daemon() {
        assert!(parse(&[]).verb.is_none());
    }

    #[test]
    fn the_daemon_verb_defaults_to_info() {
        let Some(Verb::Daemon(args)) = parse(&["daemon"]).verb else {
            panic!("the daemon verb parses to Verb::Daemon");
        };
        assert_eq!(args.log_level, super::DEFAULT_LOG_LEVEL);
    }

    #[test]
    fn the_daemon_verb_takes_a_filter_directive() {
        let Some(Verb::Daemon(args)) = parse(&["daemon", "--log-level", "mercury=debug"]).verb
        else {
            panic!("the daemon verb parses to Verb::Daemon");
        };
        assert_eq!(args.log_level, "mercury=debug");
    }

    #[test]
    fn the_log_level_is_not_a_top_level_flag() {
        assert!(Args::try_parse_from(["mercury", "--log-level", "debug"]).is_err());
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
