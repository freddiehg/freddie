# the command line is freddie's, the daemon is the app's

Every freddie app is one process that owns the keyboard, plus a handful of verbs for finding it, starting it, and stopping it. Those verbs are the same whatever the app does, and all of them now live in mercury, where a second app cannot reach them.

`freddie_cli` is a new crate holding the whole command surface. An app supplies its name, its daemon body, and whatever extra flags that body takes; it gets `start`, `restart`, `status`, `logs`, `stop`, `install`, `uninstall`, and the hidden `daemon` for free, keyed to its own name and writing to its own log file. mercury becomes an implementation of one trait and a `main` that is a single call.

The name of the binary is the fork's, not mercury's. Nothing in `freddie_cli` spells "mercury", and nothing in an app spells the verbs.

A new crate rather than a dependency, and that is settled rather than assumed: `refactors/past/reuse-existing-crates.md` audited `single-instance`, `service-manager`, and `daemonize` against what these verbs need and none of them fit. `single-instance` cannot probe without acquiring or report a pid, so `status` and `stop` have nothing to build on; `service-manager` cannot express `SuccessfulExit=false`, which is the key the daemon's exit code is tuned to. Whoever forks this gets the lifecycle from here or writes it again themselves.

## What belongs where

`freddie_cli` owns:

- the `Args`/`Verb` types and the parse
- the single-instance lock: acquiring it for the daemon, probing it for the clients
- logging setup, and the log directory
- every client verb, since all of them are the lock, the log file, and a subprocess
- delivering SIGTERM to the app as a request to stop

The app owns:

- its name, which keys the lock file, the log directory, and the help text
- what the daemon does, which is everything `mercury::daemon` holds today
- any flag beyond `--log-level` that its daemon takes
- what stopping means, because only the app knows what has to be undone first

## The seam

```rust
/// What an app is, to the command line that runs it.
///
/// One impl per binary. Everything else in this crate is generic over it.
pub trait App {
    /// The flags this app's daemon takes beyond the shared ones.
    ///
    /// [`NoArgs`] for an app that takes none.
    type Args: clap::Args + fmt::Debug;

    /// The name the lock file, the log directory, and `--help` are all keyed to.
    ///
    /// One name, so a client cannot look for the daemon under a name the daemon did not
    /// register under.
    const NAME: &'static str;

    /// The one-line description at the top of `--help`.
    const ABOUT: &'static str;

    /// Be the daemon. Returns when it has quit.
    ///
    /// Called with the lock held and logging initialized. Returning drops the lock, so an app
    /// that wants to stay running stays inside this call.
    fn run(args: &Self::Args);
}

/// The daemon flags of an app that adds none.
#[derive(clap::Args, Debug)]
pub struct NoArgs {}
```

## The generic command line

clap's derive accepts generic parameters, so the app's flags flatten into the shared `daemon` verb. Verified on the pinned 1.96.0 against clap 4.6.2: `app daemon --port 4001` parses into the flattened struct, and the shared defaults resolve.

```rust
#[derive(Parser, Debug)]
#[command(version)]
pub struct Args<A: clap::Args> {
    #[command(subcommand)]
    pub verb: Option<Verb<A>>,
}

/// What the command line asked the app to do, where `None` is the bare binary.
///
/// Each variant's doc comment is its line in `--help`, so the help text cannot drift from the
/// verbs. Declaration order is help order.
#[derive(Subcommand, Debug)]
pub enum Verb<A: clap::Args> {
    /// Run the daemon in this terminal, in the foreground.
    Daemon(DaemonArgs<A>),
}

/// What the foreground daemon can be told: what this crate asks of every app, and what the app
/// asks for itself.
#[derive(clap::Args, Debug)]
pub struct DaemonArgs<A: clap::Args> {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    ///
    /// A `tracing_subscriber` filter directive, so `info` and `mercury=debug,bind=warn` are both
    /// accepted. Only the foreground daemon has a terminal to show anything on.
    #[arg(long, env = "LOG_LEVEL", default_value = DEFAULT_LOG_LEVEL)]
    pub log_level: String,

    #[command(flatten)]
    pub app: A,
}
```

`#[command(name = ...)]` cannot carry `A::NAME`, because a derive macro reads only what is written beside it. The name is set on the built `Command` instead, which is also where `A::ABOUT` goes. Verified on the pinned 1.96.0: `--help` prints `Usage: mercury [COMMAND]` with the app's about line above it.

```rust
/// Parse this process's arguments as `A`'s command line.
///
/// `Command::name` at runtime rather than `#[command(name)]` at derive time: the name belongs to
/// the app, and the derive cannot see a constant from a type it is generic over.
fn parse<A: App>() -> Args<A::Args> {
    let command = Args::<A::Args>::command().name(A::NAME).about(A::ABOUT);
    let matches = command.get_matches();
    Args::<A::Args>::from_arg_matches(&matches)
        .expect("the derived type matches the command it derived")
}
```

`use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};`.

## The entry point

```rust
/// Do what the command line asked, and report the exit code for it.
fn dispatch<A: App>(verb: Option<Verb<A::Args>>) -> i32 {
    match verb {
        // The bare invocation. `mercury-start.md` makes this start the daemon and follow its log.
        None => daemon::run::<A>(&DaemonArgs::default_for::<A>()),
        Some(Verb::Daemon(args)) => daemon::run::<A>(&args),
    }
}

/// Be `A`'s command line. Never returns.
///
/// # Panics
///
/// Panics if the derived parser and the derived command disagree, which is a bug in this crate
/// rather than anything a caller can cause.
pub fn main<A: App>() -> ! {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    let code = dispatch::<A>(parse::<A>().verb);
    // Every path above has returned, so the daemon's locals are already dropped by the time this
    // skips the rest of the destructors.
    std::process::exit(code);
}
```

## The daemon verb

The lock and the logging move ahead of the app, so an app cannot forget either and two apps cannot disagree about where they go.

```rust
/// Run `A`'s daemon in the foreground: set up logging, take the lock, and hand over.
pub(crate) fn run<A: App>(args: &DaemonArgs<A::Args>) -> i32 {
    let log_path = logging::init(A::NAME, &args.log_level);
    println!("{}: logging to {}", A::NAME, log_path.display());

    // Before anything that touches the machine. Two instances swallow and re-emit each other's
    // keys forever, which wedges the keyboard. Held for the length of this call, so the lock
    // outlives the daemon body.
    let _instance = match freddie_single_instance::acquire(A::NAME) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("{}: {e}", A::NAME);
            error!(error = %e, "another instance holds the lock; not starting");
            return 1;
        }
    };

    A::run(&args.app);
    0
}
```

`logging::init` gains the app name and derives `~/Library/Logs/<name>/<name>.log` from it, replacing the two `mercury` literals in `crates/mercury/src/logging.rs`. Everything else in that module moves across unchanged.

## Stopping stays the app's

Superseded by `freddie-daemon-runtime.md`, which makes the stop request a `From<Stop>` bound on the app's event type and installs the handler itself. The `on_stop` helper below is what this doc proposed before that existed; it is kept because the reasoning for why stopping cannot be the shared crate's is unchanged.

`freddie_cli` delivers the request; the app decides what it means. mercury's answer is the event it already has: `quit_event()` opens the modifiers a command layer swallowed and pushes `Kill`, and no shared crate can know that has to happen.

In `freddie_cli`, for an app to call from inside its own runtime:

```rust
/// Call `on_stop` when the process is asked to terminate.
///
/// `launchctl bootout` and `<app> stop` both send SIGTERM. This routes it to the app rather than
/// acting on it, because the way out is the app's: only it knows what its model has to undo before
/// the process may go.
///
/// Requires a tokio runtime with the signal driver enabled, and so is called from inside the
/// app's `run`. A failure to install is logged and not fatal: the app runs, and a terminated
/// process simply does not get the graceful path.
pub fn on_stop(on_stop: impl Fn() + Send + 'static) {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: stopping");
                    on_stop();
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated process will not stop gracefully");
        }
    }
}
```

mercury's call, in `serve`:

```rust
    freddie_cli::on_stop({
        let event_tx = event_tx.clone();
        move || {
            let _ = event_tx.send(quit_event());
        }
    });
```

`mercury-stop.md`'s change 1 becomes this call rather than the handler written out there, and its account of what installing the handler costs applies unchanged.

## What mercury becomes

`crates/mercury/src/main.rs`, entire:

```rust
//! The mercury binary: a freddie app and the command line that runs it.

use freddie_cli::App;

mod daemon;

/// The loopback port the event socket listens on, and nothing else mercury needs from a flag.
#[derive(clap::Args, Debug)]
pub struct MercuryArgs {
    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}

struct Mercury;

impl App for Mercury {
    type Args = MercuryArgs;
    const NAME: &'static str = "mercury";
    const ABOUT: &'static str = "A layered keyboard remapper.";

    fn run(args: &Self::Args) {
        daemon::run(args.port);
    }
}

fn main() -> ! {
    freddie_cli::main::<Mercury>()
}
```

`crates/mercury/src/cli.rs` and `crates/mercury/src/logging.rs` are deleted; their contents live in `freddie_cli`. `crates/mercury/src/daemon.rs` keeps everything it holds today except the logging call and the lock, which `freddie_cli` does before calling it, so `pub(crate) fn run(port: u16)` starts at the `freddie_windows::init()` line.

## The changes, in order

1. **`freddie_cli` with the daemon verb only.** The trait, the generic `Args`/`Verb`/`DaemonArgs`, `parse`, `main`, `dispatch`, `daemon::run`, and `logging` moved across and keyed to `A::NAME`. mercury shrinks to the `main.rs` above. No verb is added and no behaviour changes: `mercury`, `mercury daemon`, and `mercury daemon --log-level` do exactly what they do now, and `mercury --help` prints the same thing.
2. **`on_stop`.** The signal helper, and mercury calling it. This is `mercury-stop.md`'s change 1, landing in the shared crate.
3. **The client verbs.** `start`, `restart`, `status`, `logs`, `stop`, `install`, and `uninstall`, moved into `freddie_cli`'s own `client` module and keyed to `A::NAME` rather than to a `const APP` of mercury's.

   `install` and `uninstall` generalize without a decision to make: the launchd label is already `format!("hg.freddie.{APP}")`, the plist path is derived from it, and the `Agent` struct's program comes from `current_exe`. Only the `hg.freddie.` prefix is a constant worth a second look, since a fork that is not freddie-derived may want its own.

## The verbs mercury already has by then

`mercury-stop.md`, `mercury-status-and-logs.md`, and `mercury-start.md` ship first, into `crates/mercury/src/client.rs` as they are written. This doc moves that file across.

The move is mechanical, and it is the reason those docs put every verb in one module and keyed all of them to a single `APP` constant. `client.rs` becomes `freddie_cli`'s client module, `const APP: &str = "mercury"` becomes `A::NAME` at each of its use sites, and each verb's function gains the `<A: App>` its name lookup now needs. No verb changes what it does, what it prints, or what it exits with.

What does change is the `Verb` enum those docs each add a variant to: the variants move onto the generic `Verb<A>` here, and `dispatch` gains their arms. The parse tests move with them.
