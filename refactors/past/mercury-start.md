# starting a mercury that outlives the terminal

`mercury` starts a daemon detached from the terminal that started it, says which pid it is, and exits. `mercury start` is the same thing spelled out. `mercury restart` replaces whatever was running with a fresh one, which is what a rebuild wants.

Nothing follows the log on its own. `mercury logs` does that, against a daemon that is already up.

Follows `refactors/past/mercury-daemon-verb.md` for the verb to spawn, `refactors/past/single-instance-holder.md` for finding what is already running, `refactors/past/mercury-status-and-logs.md` for the follower, and `refactors/past/mercury-stop.md` for the stopping half of `restart`.

## What each verb guarantees

- `mercury start` — a daemon is running when this returns. It does not matter whether this call started it.
- `mercury restart` — a daemon is running when this returns, and it is not the one that was there.
- `mercury restart --force` — the same, destroying the old one rather than asking it to quit. `--force` means what it means on `stop`: SIGKILL instead of SIGTERM.
- `mercury` — exactly `mercury start`. The bare command is the short spelling of the thing you run most.
- `mercury daemon` — be the daemon in this process. Hidden from `--help`; it is what `start` spawns and what `launch-at-login.md` puts in its plist.

`restart` from cold starts a daemon rather than failing, so `cargo build -p mercury && mercury restart` does not have to know whether one was up. The only way `restart` fails is a daemon that will not let go of the lock, and then it starts nothing: the lock would refuse the new one, and the message would be about the wrong process.

Ctrl-C reaches a follower and never a daemon. `start` spawns into a process group of its own, and nothing runs a daemon in a terminal's foreground group, so the SIGINT that ends a `mercury logs` cannot reach the process holding the keyboard.

## Detaching without `unsafe`

The daemon is spawned as `current_exe daemon` in a process group of its own.

`current_exe` rather than a configured path, so the child is whatever binary the user invoked: `cargo run -p mercury` starts the debug build under `target/`, and an installed binary starts itself, with no path to keep in step.

`process_group(0)` is what makes Ctrl-C on the log stream not take the daemon with it. Without it the child stays in the terminal's foreground process group and receives the same SIGINT the follower does. It also keeps the daemon off the SIGHUP the kernel sends the session's foreground group when the terminal closes. It is a safe method on `std::os::unix::process::CommandExt`, so this costs no `unsafe` and no `libc`, which matters against a workspace that forbids the former. Verified on the pinned 1.96.0.

Not a full session leader: that needs `setsid`, which macOS does not ship as a binary and which is an unsafe call from Rust. A process group is enough for both signals that would otherwise reach it, and `launch-at-login.md` is the answer for a daemon that should outlive the login session entirely.

## Change 1: `mercury start`

In `client.rs`.

```rust
/// How long `start` waits for a spawned daemon to take the lock.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// A daemon that is up, and whether this call is why.
enum Running {
    /// Already running when this looked, and it had named itself.
    Adopted(Pid),
    /// Already running when this looked, mid-acquire and not yet named.
    AdoptedUnnamed,
    /// Spawned by this call.
    Started(Pid),
}

/// Why no daemon is running.
enum NotStarted {
    /// The lock could not be read, so nothing is known about what holds it.
    Unreadable(LockError),
    /// The daemon could not be spawned.
    Unspawnable(io::Error),
    /// It was spawned and never took the lock. Its own account is in the log.
    Silent(Pid),
}

/// The terminal wording for each, without the `mercury: ` a caller puts in front.
impl fmt::Display for NotStarted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "{e}"),
            Self::Unspawnable(e) => write!(f, "could not start the daemon: {e}"),
            Self::Silent(pid) => write!(
                f,
                "the daemon (pid {pid}) started and stopped without taking the lock"
            ),
        }
    }
}

/// Make sure a daemon is running, starting one if none is.
///
/// The check before spawning is for the answer, not for mutual exclusion. Two `start`s at the same
/// instant can both see [`Held::Free`] and both spawn, and the lock refuses one of the two daemons
/// exactly as it refuses a second `mercury daemon`; nothing here has to be atomic.
///
/// Reports facts and says nothing to the terminal, as `stop_daemon` does, because `start` and
/// `restart` word the outcome differently.
fn ensure_started() -> Result<Running, NotStarted> {
    match freddie_single_instance::holder(APP) {
        Ok(Held::By(pid)) => return Ok(Running::Adopted(pid)),
        Ok(Held::Unnamed) => return Ok(Running::AdoptedUnnamed),
        Ok(Held::Free) => {}
        Err(error) => {
            debug!(%error, "could not read the lock");
            return Err(NotStarted::Unreadable(error));
        }
    }
    let pid = spawn_daemon().map_err(|error| {
        debug!(%error, "could not spawn the daemon");
        NotStarted::Unspawnable(error)
    })?;
    debug!(daemon = %pid, "spawned the daemon");
    if wait_until_held() {
        Ok(Running::Started(pid))
    } else {
        debug!(daemon = %pid, timeout = ?START_TIMEOUT, "the daemon never took the lock");
        Err(NotStarted::Silent(pid))
    }
}

/// Spawn this same binary as `mercury daemon`, detached from this terminal.
///
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer then has nowhere
/// to write, which is why `--log-level` is not passed through: it governs a terminal this child
/// does not have. The log file records `debug` regardless, and `mercury logs` reads that.
fn spawn_daemon() -> io::Result<Pid> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(Pid(child.id()))
}

/// Poll until something holds the lock, up to [`START_TIMEOUT`]. `true` when one does.
///
/// Polled rather than waited on, unlike `watch_for_free`: flock reports a release, so waiting for
/// a daemon to go is edge-triggered, and there is no way to wait on another process *taking* a
/// lock. This is the one direction that has no edge.
///
/// Taking the lock is the readiness signal, and the daemon takes it first thing, before it
/// measures the screens, shows an icon, or grabs the keyboard. So this returning `true` says the
/// process is alive and is the one mercury, not that the keyboard is grabbed: a daemon refused
/// Accessibility fails a moment later and says so in the log.
fn wait_until_held() -> bool {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if !matches!(freddie_single_instance::holder(APP), Ok(Held::Free)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// Say what `ensure_started` found, and report the exit code for it.
///
/// Shared by `start` and `restart`, so a started daemon reads the same whichever verb produced it.
fn report(running: Result<Running, NotStarted>) -> i32 {
    match running {
        Ok(Running::Started(pid)) => {
            info!("mercury started (pid {pid})");
            0
        }
        Ok(Running::Adopted(pid)) => {
            info!("mercury is already running (pid {pid})");
            0
        }
        Ok(Running::AdoptedUnnamed) => {
            info!("mercury is already running (it has not recorded its pid yet)");
            0
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            1
        }
    }
}

/// `mercury start`: make sure a daemon is up, and do not stay to watch.
pub(crate) fn start() -> i32 {
    logging::init(&Terminal::Client);
    report(ensure_started())
}
```

Imports gained: `std::process::Stdio` is already there for `logs`.

`Verb` gains `Start`, and `main.rs` the arm calling `client::start()`.

## Change 2: `mercury restart`

```rust
/// What `mercury restart` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct RestartArgs {
    /// Destroy the running daemon with SIGKILL instead of asking it to quit.
    ///
    /// For a daemon that no longer answers. It runs no destructors, so a modifier the command
    /// layer swallowed is left down in whatever app was in front.
    #[arg(long)]
    pub force: bool,
}
```

```rust
/// `mercury restart`: replace the running daemon with a fresh one.
///
/// The two halves are already sequenced by the lock. `stop_daemon` returns only once the lock is
/// free, which is the same condition `ensure_started` needs to find, so the new daemon never races
/// the old one's shutdown and reports "already running" against the process it just replaced.
///
/// A daemon that would not stop means no start is attempted: the old process still owns the tap,
/// and spawning a second one that the lock immediately refuses would say nothing useful.
///
/// Starting from cold is a restart with an empty first half rather than an error, so a script that
/// restarts after a rebuild does not have to know whether anything was up.
pub(crate) fn restart(args: &RestartArgs) -> i32 {
    logging::init(&Terminal::Client);
    let signal = if args.force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(signal) {
        Ok(Some(pid)) => info!("mercury stopped (pid {pid})"),
        Ok(None) => debug!("nothing was running to stop"),
        Err(failure) => {
            warn!("mercury: not restarting: {failure}");
            return 1;
        }
    }
    report(ensure_started())
}
```

`stop`'s own `logging::init` moves into `stop` proper, so `stop_daemon` initializes nothing and `restart` can call it after its own init.

This is the verb a rebuild wants: `cargo build -p mercury && ./target/debug/mercury restart` replaces a running daemon with the binary just built. It stops and starts rather than signalling the daemon to re-exec itself, because the running process is the old binary and nothing about it knows a new one exists.

## Change 3: the bare `mercury` starts a daemon

`main.rs`, where the bare verb has run the daemon in the foreground until now:

```diff
 fn run(verb: Option<Verb>) -> i32 {
     match verb {
-        None => {
-            daemon::run(&DaemonArgs::default());
-            0
-        }
+        None | Some(Verb::Start) => client::start(),
+        Some(Verb::Restart(args)) => client::restart(&args),
         Some(Verb::Status) => client::status(),
```

One arm for both, because they are one behaviour rather than two that happen to agree.

`DaemonArgs::default()` and its test go with it: nothing constructs `DaemonArgs` outside clap once the bare `mercury` no longer runs a daemon in this process.

## Change 4: `daemon` leaves the help

```diff
+    /// Run the daemon in this process. Not for typing: `mercury start` spawns it.
+    #[command(hide = true)]
     Daemon(DaemonArgs),
```

The finished `Verb`, in help order:

```rust
pub enum Verb {
    /// Start the daemon if it is not running, and exit.
    Start,
    /// Stop the running daemon and start a fresh one.
    Restart(RestartArgs),
    /// Report whether the daemon is running, and its pid.
    Status,
    /// Follow the log, starting nothing.
    Logs(LogsArgs),
    /// Ask the running daemon to quit.
    Stop(StopArgs),
    /// Run the daemon in this process. Not for typing: `mercury start` spawns it.
    #[command(hide = true)]
    Daemon(DaemonArgs),
}
```

Tests:

```rust
#[test]
fn the_lifecycle_verbs_parse() {
    assert!(matches!(parse(&["start"]).verb, Some(Verb::Start)));
    assert!(matches!(parse(&["restart"]).verb, Some(Verb::Restart(_))));
}

#[test]
fn restart_is_gentle_by_default() {
    let Some(Verb::Restart(args)) = parse(&["restart"]).verb else {
        panic!("the restart verb parses to Verb::Restart");
    };
    assert!(!args.force);
}

#[test]
fn restart_takes_force() {
    let Some(Verb::Restart(args)) = parse(&["restart", "--force"]).verb else {
        panic!("the restart verb parses to Verb::Restart");
    };
    assert!(args.force);
}

// Hidden is not gone: `start` spawns it and the launch agent runs it.
#[test]
fn the_daemon_verb_still_parses() {
    assert!(matches!(parse(&["daemon"]).verb, Some(Verb::Daemon(_))));
}

```

## Verifying

From a shell with no mercury running:

- `mercury` says "mercury started (pid N)" and returns to the shell. `mercury status` reports pid N, and the keyboard remaps.
- `mercury` again says "mercury is already running (pid N)" and returns.
- Closing the terminal entirely leaves the daemon up, by `mercury status` from a new one.
- `mercury logs` against it follows; Ctrl-C ends the follow and `mercury status` still reports pid N.
- `mercury restart` says "mercury stopped (pid N)" then "mercury started (pid M)" with `M` a different pid. The log shows the old daemon's `SIGTERM: quitting` and `kill: exiting` before the new one's `initial state`, with the two pid stamps distinguishing them.
- `mercury restart` with nothing running starts one and says only "mercury started (pid N)".
- `mercury restart --force` ends the old daemon with no `kill: exiting` line and starts a new one.
- Editing a binding, then `cargo build -p mercury && ./target/debug/mercury restart`, and the new binding is live without any window having been touched.
- `mercury --help` does not list `daemon`, and `mercury daemon` still runs one.
- `tccutil reset Accessibility` and then `mercury`: it reports a started pid, because taking the lock is what it waits for, and the log carries the `could not intercept the keyboard` line the daemon wrote a moment later.

## What `launch-at-login.md` inherits

Its plist runs `mercury daemon`, which stays invocable for exactly this reason, and `launchctl bootout` is a clean shutdown through the SIGTERM handling `mercury-stop.md` landed. That doc's open question about a hand-started mercury fighting the loaded agent is answered by the lock: `mercury start` finds the agent's daemon holding it and adopts it.
