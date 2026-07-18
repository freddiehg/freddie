# running mercury in the background

There is one mercury, it runs detached, and the terminal is a client of it. `mercury` starts the daemon if it is not up and then follows its log; `mercury start`, `mercury logs`, `mercury stop`, and `mercury status` each do one of those pieces on its own. Ctrl-C ends the log stream and leaves the daemon running.

This is the prefactor for `launch-at-login.md`. That doc's plist runs a binary that must stay in the foreground of its own process and quit cleanly on `launchctl bootout`, which is `mercury daemon` plus SIGTERM handling. Once both exist the plist's `ProgramArguments` becomes `[/usr/local/bin/mercury, daemon]` and nothing else about it changes.

## The command surface

```
mercury: a keyboard remapper

    mercury           start the daemon if it is not running, then follow its log
    mercury start     start the daemon if it is not running, and exit
    mercury restart   stop the running daemon and start a fresh one
    mercury logs      follow the log, starting nothing
    mercury stop      ask the running daemon to quit
    mercury status    report whether the daemon is running, and its pid
    mercury daemon    run the daemon in this terminal, in the foreground
```

No flags and no values, on any verb. clap owns the surface, so that listing is `mercury --help` rather than a string kept beside the parser, `-V` prints the version, and an unknown verb exits 2 with a suggestion. Verified against clap 4.6 on the pinned 1.96.0.

Exit codes: 0 for success, 1 when the operation failed, 2 for an unparseable command line. `mercury status` exits 1 when no daemon is running, so a script can test it. `mercury stop` exits 0 when no daemon is running, so calling it twice is not an error, and `mercury restart` starts one when none was running rather than refusing.

`restart` is the verb a rebuild wants: `cargo build -p mercury && ./target/debug/mercury restart` replaces a running daemon with the binary just built. It stops and starts rather than signalling the daemon to re-exec itself, because the running process is the old binary and nothing about it knows the new one exists.

## Why `stop` is a signal

`external-events.md` defines the loopback socket mercury listens on, and states that `IncomingEvent` names exactly what an outside sender may say so that "remote key injection and remote quit are unrepresentable rather than filtered". A quit frame is the thing that vocabulary exists to exclude, so `stop` does not go over the socket, and the socket does not grow a variant for it.

SIGTERM instead. The daemon turns it into the same `MercuryEvent` the menu bar's Quit sends, so the way out is the one that already exists: `quit` is bound at the root, it opens the held modifiers and pushes `Kill`, the effect loop breaks, and the `Interceptor` releases the keyboard on the way out. A mercury killed without that handler dies where it stands, and `refactors/past/single-instance.md` records the observed consequence: "mercury installs no signal handler, so its `Drop` impls do not run: the log has a run with no `kill: exiting` line for exactly that reason."

## The lock file carries its holder's pid

`stop` needs to know which process to signal. `freddie_single_instance` already owns the fact of whether a mercury is running, so it also reports which one: after taking the lock, the holder writes its pid into the locked file.

The lock is the liveness signal and the contents are only an address. A pid is read exactly when the lock is refused, so a pid left behind by a process that has since died is never reported as live: that file's lock is free, and the probe answers `Free` without looking inside. This is what keeps the design clear of the stale-pid-file failure the single-instance doc was written to avoid. Verified on the pinned 1.96 toolchain against a standalone binary: with the lock held the probe reads `By(Pid(73919))`; after the holder drops, the probe reads `Free` while the file still contains `73919`.

There is a window between taking the lock and writing the pid in which the file is empty, and `Held::Unnamed` is that window rather than something a caller has to infer from an empty string.

## Change 1: mercury quits on SIGTERM

`crates/mercury/Cargo.toml`:

```diff
-tokio = { version = "1", features = ["rt", "macros", "sync", "time"] }
+tokio = { version = "1", features = ["rt", "macros", "signal", "sync", "time"] }
```

`crates/mercury/src/main.rs`, in `run`, after the `freddie_app_nav` watcher is installed and before the `select!`:

```rust
    // `launchctl bootout` and `mercury stop` both send SIGTERM. Route it into the event channel as
    // the same Quit the menu bar sends, so a terminated mercury leaves the way it would have on its
    // own: the model turns it into `Kill`, the effect loop breaks, and the `Interceptor` releases
    // the keyboard. Without this the process dies where it stands, holding the grab until the
    // kernel tears it down and never re-opening the modifiers a command layer swallowed.
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: quitting");
                    let _ = event_tx.send(quit_event());
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated mercury will not release the keyboard");
        }
    }
```

A spawned task rather than a third `select!` arm, because a `select!` arm that completed would drop the other two futures and skip the graceful path this exists to run.

The runtime is already `new_current_thread().enable_all()`, which drives the signal handler. Confirmed to build and run on the pinned 1.96.0.

Verifying: `cargo run -p mercury`, then `kill <pid>` from another pane. `~/Library/Logs/mercury/mercury.log` gets a `SIGTERM: quitting` line followed by `kill: exiting`, and the keyboard is normal afterwards.

## Change 2: the log path without a subscriber

`mercury logs` needs the path and must not install a subscriber or create the file.

`crates/mercury/src/logging.rs`, before:

```rust
pub fn init() -> PathBuf {
    let dir = log_dir();
    // ...
    dir.join(LOG_FILE)
}
```

After:

```rust
/// Where the log file is, whether or not tracing has been initialized or anything has been written.
///
/// Separate from [`init`] because `mercury logs` needs the path in a process that installs no
/// subscriber of its own: the daemon owns the log, and a client only reads it.
#[must_use]
pub fn log_path() -> PathBuf {
    log_dir().join(LOG_FILE)
}

pub fn init() -> PathBuf {
    let dir = log_dir();
    // ... unchanged ...
    log_path()
}
```

## Change 3: `freddie_single_instance` reports its holder

The module doc, before:

```rust
//! The file is a rendezvous name rather than storage. Nothing is written to it or read
//! from it, so neither its contents nor its existence mean anything; only the lock
//! does. That is what makes a leftover file from the last run the normal case rather
//! than a stale artifact to detect and clean up.
```

After:

```rust
//! The lock is the only thing that means anything. Whether the file exists, and what it
//! contains, mean nothing on their own, which is what makes a leftover file from the last
//! run the normal case rather than a stale artifact to detect and clean up.
//!
//! The holder writes its pid into the file so [`holder`] can say which process is running.
//! That pid is read only when the lock is refused, so a pid belonging to a process that has
//! since died is never reported: its file's lock is free, and the probe answers
//! [`Held::Free`] without reading. A pid here is an address for a process already known to
//! be alive, never the evidence that it is.
```

New items:

```rust
/// A process id, as the operating system numbers processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pid(pub u32);

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Who holds a lock, at the moment of asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// Nobody. Whatever the file contains belongs to a run that has ended.
    Free,
    /// A live process, which recorded which one it is.
    By(Pid),
    /// A live process that has taken the lock and not yet written its pid.
    Unnamed,
}
```

`acquire_at` splits, so that a probe never writes:

```rust
/// Open `path` and try to take its lock, creating the parent directory if it is missing.
///
/// Shared by [`acquire_at`], which keeps the file and records its pid in it, and [`holder_at`],
/// which drops it immediately and has only asked a question.
fn lock(path: &Path) -> Result<File, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
    }
    // `read(true)` is new alongside the existing write: the pid is read back through this same
    // open mode in tests, and a write-only handle cannot serve that.
    // `truncate(false)` still: the lock must not disturb the file before it is held, and the
    // holder truncates deliberately in `record_pid` once it is.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Unavailable)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::AlreadyRunning(path.to_owned())),
        Err(std::fs::TryLockError::Error(e)) => Err(LockError::Unavailable(e)),
    }
}

/// Write this process's pid over whatever the file held.
///
/// `set_len` before the write, because the previous run's pid may be longer than this one's, and
/// writing a short number over a long one leaves trailing digits that parse as a pid belonging to
/// nobody.
fn record_pid(mut file: &File) -> io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(std::process::id().to_string().as_bytes())?;
    file.flush()
}

/// The pid the file at `path` names, or `None` when it holds nothing that reads as one.
///
/// Meaningful only while the lock is held; see [`holder_at`].
fn read_pid(path: &Path) -> Option<Pid> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok().map(Pid)
}
```

`acquire_at`, after:

```rust
/// Claim `path` for this process, or report that another process holds it.
///
/// `try_lock` rather than `lock`: a second instance is refused immediately instead of blocking, so
/// a caller that cannot run is told so rather than left waiting for a process that may never exit.
///
/// An `Instance` means the lock is held and the pid is recorded, both or neither. Failing to write
/// the pid fails the acquire, rather than handing back a lock nobody can address: the file is open
/// and writable by the time we are holding its lock, so a failure here is the disk going away, and
/// a mercury that cannot be found by `mercury stop` is not one worth starting.
///
/// # Errors
///
/// Returns [`LockError::AlreadyRunning`] when another process holds the lock, and
/// [`LockError::Unavailable`] when the file cannot be created, opened, locked, or written.
pub fn acquire_at(path: &Path) -> Result<Instance, LockError> {
    let file = lock(path)?;
    record_pid(&file).map_err(LockError::Unavailable)?;
    Ok(Instance { _file: file })
}
```

`Instance` keeps its `_file` name. The field is never read (the lock lives on the open file description, not on anything we call), and the workspace denies `unused`, so naming it `file` fails the build.

The probe:

```rust
/// Who holds `app`'s lock right now.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory, and
/// otherwise whatever [`holder_at`] returns.
pub fn holder(app: &str) -> Result<Held, LockError> {
    holder_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Who holds `path` right now, found by trying to take the lock and reading the file when that is
/// refused.
///
/// Taking it is the proof that nobody else had it, and the lock is released again before this
/// returns. So the answer describes the instant it was asked and a process may start or exit
/// immediately afterwards. Callers act on it knowing that; [`acquire`] remains the only thing that
/// decides who runs.
///
/// # Errors
///
/// Returns [`LockError::Unavailable`] when the file cannot be created, opened, or locked.
pub fn holder_at(path: &Path) -> Result<Held, LockError> {
    match lock(path) {
        // Dropping the file here closes it, which releases the lock we just took.
        Ok(_probe) => Ok(Held::Free),
        Err(LockError::AlreadyRunning(_)) => Ok(read_pid(path).map_or(Held::Unnamed, Held::By)),
        Err(e) => Err(e),
    }
}
```

Imports gained: `std::io::{Read, Seek, SeekFrom, Write}`.

Tests, added to the existing module and using its `temp_lock` helper:

```rust
#[test]
fn the_holder_is_named_by_pid() {
    let path = temp_lock("holder-pid");
    let _held = acquire_at(&path).expect("the path is free");
    assert_eq!(holder_at(&path).expect("probing"), Held::By(Pid(std::process::id())));
}

#[test]
fn an_unlocked_path_is_free() {
    let path = temp_lock("holder-free");
    assert_eq!(holder_at(&path).expect("probing"), Held::Free);
}

// The property the whole design rests on: a pid outlives its process in the file, and is never
// reported once the lock behind it is gone.
#[test]
fn a_released_lock_is_free_though_its_pid_remains() {
    let path = temp_lock("holder-stale");
    let held = acquire_at(&path).expect("the path is free");
    drop(held);
    assert_eq!(holder_at(&path).expect("probing"), Held::Free);
    let left = std::fs::read_to_string(&path).expect("the file outlives the lock");
    assert_eq!(left.trim(), std::process::id().to_string());
}

// A probe must not stamp itself into a file it only asked about, or every `mercury status` would
// leave a dead pid behind for the next reader.
#[test]
fn probing_writes_nothing() {
    let path = temp_lock("holder-readonly");
    assert_eq!(holder_at(&path).expect("probing"), Held::Free);
    assert!(std::fs::read_to_string(&path).expect("the probe created it").is_empty());
}

// A longer pid from a previous run must not leave a tail behind a shorter one.
#[test]
fn a_recorded_pid_replaces_the_whole_file() {
    let path = temp_lock("holder-truncate");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(&path, "4294967295").expect("a longer pid from an earlier run");
    let _held = acquire_at(&path).expect("the path is free");
    let written = std::fs::read_to_string(&path).expect("reading it back");
    assert_eq!(written, std::process::id().to_string());
}
```

## Change 4: the command line, with one verb

Pure prefactor: `mercury` and `mercury daemon` both do exactly what `mercury` does today. Each later change adds its own variant to `Command` and its own arm to the match.

`crates/mercury/src/main.rs` keeps `main`, the usage text, and the parser. Everything else in the file today (`run`, `run_event_loop`, `dispatch_event`, `run_effect_loop`, `perform_effect`, `schedule_timer`, `place_window`, `foreground_app`, and the module doc describing the threads) moves verbatim into a new `crates/mercury/src/daemon.rs`, with today's `fn main` body becoming `pub(crate) fn run()`. `mod logging;` stays in `main.rs`, and `daemon.rs` reaches it as `crate::logging`.

`clap` with its `derive` feature is already a mercury dependency by the time this lands, so nothing here adds it.

```rust
//! The mercury command line.
//!
//! One process runs the model and owns the keyboard (`mercury daemon`, in `daemon.rs`); every
//! other verb is a client that finds it through its lock and reads the log it writes.

use clap::{Parser, Subcommand};

mod daemon;
mod logging;

#[derive(Parser)]
#[command(name = "mercury", version, about = "A keyboard remapper")]
struct Cli {
    #[command(subcommand)]
    verb: Option<Verb>,
}

/// What the command line asked for, where `None` is the bare `mercury`.
///
/// `Copy`, because it carries nothing: a fieldless enum passed by value trips
/// `clippy::needless_pass_by_value` under the workspace's `pedantic`, and being `Copy` is both the
/// right answer and the one that keeps `run` taking it by value.
///
/// Each variant's doc comment is its line in `mercury --help`, so the help text cannot drift from
/// the verbs the way a hand-maintained usage string does.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    /// Run the daemon in this terminal, in the foreground.
    Daemon,
}

/// Do what was asked, and report the exit code for it.
fn run(verb: Option<Verb>) -> i32 {
    match verb {
        // The bare `mercury`. Change 7 makes this start the daemon and follow its log; until then
        // it is what running mercury has always been.
        None | Some(Verb::Daemon) => {
            daemon::run();
            0
        }
    }
}

fn main() {
    // `parse` exits 2 itself on a bad command line, after printing the usage, so there is no error
    // arm here. Tests reach the parser through `try_parse_from`, which returns instead.
    let code = run(Cli::parse().verb);
    // Every path above has returned, so the daemon's locals (the lock, the menu bar, the run loop
    // stopper) are already dropped by the time this skips the rest of the destructors.
    std::process::exit(code);
}
```

Tests, in `main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{Cli, Verb};
    use clap::Parser;

    fn verb_of(args: &[&str]) -> Option<Verb> {
        Cli::try_parse_from(std::iter::once("mercury").chain(args.iter().copied()))
            .expect("a valid command line")
            .verb
    }

    #[test]
    fn no_verb_runs_the_daemon() {
        assert_eq!(verb_of(&[]), None);
    }

    #[test]
    fn the_daemon_verb_runs_the_daemon() {
        assert_eq!(verb_of(&["daemon"]), Some(Verb::Daemon));
    }

    #[test]
    fn an_unknown_verb_is_refused() {
        assert!(Cli::try_parse_from(["mercury", "frobnicate"]).is_err());
    }

    #[test]
    fn no_verb_takes_an_argument() {
        assert!(Cli::try_parse_from(["mercury", "daemon", "--now"]).is_err());
    }
}
```

Each later change extends these: a `Verb` variant carrying its help line, its `run` arm, and a test asserting it parses. Verified on the pinned 1.96.0 against clap 4.6: the derived code is clean under the workspace's `deny` on clippy `all`, `pedantic`, `nursery`, and `cargo`, given the `Copy` above.

## Change 5: `status` and `stop`

New file `crates/mercury/src/cli.rs`, declared `mod cli;` in `main.rs`.

```rust
//! The client verbs: everything `mercury` can do that is not being the daemon.
//!
//! None of these takes the single-instance lock. They probe it to find the daemon, and the daemon
//! is the only process that holds it.

use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};

/// The app name the lock is keyed to. The daemon acquires this; the clients probe it.
pub(crate) const APP: &str = "mercury";

/// How often the wait loops re-probe the lock.
const POLL: Duration = Duration::from_millis(50);

/// How long `stop` waits for the daemon to release the lock before reporting that it has not.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait out [`Held::Unnamed`], the window between a daemon locking the file and
/// writing its pid into it.
const PID_TIMEOUT: Duration = Duration::from_millis(500);

/// What a client found when it went looking for the daemon.
enum Target {
    /// Nothing holds the lock.
    NotRunning,
    /// The daemon, ready to be signalled.
    Running(Pid),
    /// Something holds the lock and never recorded a pid, so there is nothing to signal.
    Anonymous,
}

/// Report whether the daemon is running, and which process it is.
pub(crate) fn status() -> i32 {
    match freddie_single_instance::holder(APP) {
        Ok(Held::Free) => {
            println!("mercury is not running");
            1
        }
        Ok(Held::By(pid)) => {
            println!("mercury is running (pid {pid})");
            0
        }
        Ok(Held::Unnamed) => {
            println!("mercury is running (it has just started and has not recorded its pid)");
            0
        }
        Err(e) => {
            eprintln!("mercury: {e}");
            1
        }
    }
}

/// What [`stop_daemon`] found and did.
///
/// `stop` and `restart` both need the outcome and word it differently, so the stopping reports the
/// fact and each caller phrases its own line. The failures are worded here, because a daemon that
/// would not go away reads the same whichever verb asked.
enum Stopped {
    /// A daemon was signalled and let go of the lock.
    Was(Pid),
    /// There was nothing to stop.
    NotRunning,
    /// A daemon is there and did not go away, or could not be found to signal.
    Failed,
}

/// Ask the running daemon to quit, and wait for it to let go of the lock.
fn stop_daemon() -> Stopped {
    let pid = match find_daemon() {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Stopped::NotRunning,
        Ok(Target::Anonymous) => {
            eprintln!("mercury: something holds the lock but recorded no pid; it is starting or shutting down");
            return Stopped::Failed;
        }
        Err(e) => {
            eprintln!("mercury: {e}");
            return Stopped::Failed;
        }
    };
    if let Err(e) = terminate(pid) {
        eprintln!("mercury: could not signal pid {pid}: {e}");
        return Stopped::Failed;
    }
    if wait_until_free() {
        Stopped::Was(pid)
    } else {
        eprintln!("mercury: pid {pid} still holds the lock; run `kill -9 {pid}` if it is wedged");
        Stopped::Failed
    }
}

/// `mercury stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop() -> i32 {
    match stop_daemon() {
        Stopped::Was(pid) => {
            println!("mercury stopped (pid {pid})");
            0
        }
        Stopped::NotRunning => {
            println!("mercury is not running");
            0
        }
        Stopped::Failed => 1,
    }
}

/// Find the daemon, waiting out the window in which it holds the lock without having named itself.
///
/// That window is a `set_len` and a `write_all` on an already-open file, so it closes in
/// microseconds. A holder that stays anonymous past [`PID_TIMEOUT`] is one whose pid write failed,
/// which `acquire_at` treats as a failure to start, so its lock is about to be released anyway.
fn find_daemon() -> Result<Target, LockError> {
    let deadline = Instant::now() + PID_TIMEOUT;
    loop {
        match freddie_single_instance::holder(APP)? {
            Held::Free => return Ok(Target::NotRunning),
            Held::By(pid) => return Ok(Target::Running(pid)),
            Held::Unnamed if Instant::now() >= deadline => return Ok(Target::Anonymous),
            Held::Unnamed => std::thread::sleep(POLL),
        }
    }
}

/// Send SIGTERM to `pid`, which is `/bin/kill`'s default signal.
///
/// A subprocess rather than a `kill(2)` binding, because the workspace forbids `unsafe` and every
/// binding for it is an unsafe extern call. The same trade `freddie_app_nav` makes by foregrounding
/// an app through `open`. An absolute path, so `PATH` cannot point this at something else.
fn terminate(pid: Pid) -> io::Result<()> {
    let status = Command::new("/bin/kill").arg(pid.to_string()).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("/bin/kill exited with {status}")))
    }
}

/// Poll until nothing holds the lock, up to [`STOP_TIMEOUT`]. `true` when it was released.
///
/// The lock is what the wait is for, rather than the process disappearing: the daemon releases it
/// as it exits, and a released lock is exactly the condition under which the next `mercury start`
/// will succeed.
fn wait_until_free() -> bool {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if matches!(freddie_single_instance::holder(APP), Ok(Held::Free)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}
```

`daemon::run` takes its lock through `freddie_single_instance::acquire(cli::APP)` rather than a second `"mercury"` literal.

`stop` does not escalate to SIGKILL. A daemon that ignores SIGTERM is one whose event loop is wedged, and killing it uncleanly leaves exactly the state the graceful path exists to avoid, so the decision to do it is the user's and the message says how.

`USAGE`, `Command`, `parse`, and `run` gain `status` and `stop`.

Verifying: `mercury status` with nothing running prints "not running" and exits 1; `cargo run -p mercury daemon` in another pane, then `mercury status` prints the pid, `mercury stop` prints "mercury stopped", and the daemon's log ends with `SIGTERM: quitting` and `kill: exiting`.

## Change 6: `logs`

```rust
/// Where `tail` lives. Absolute, so `PATH` cannot point this at something else.
const TAIL: &str = "/usr/bin/tail";

/// How much of the existing log to print before following.
const TAIL_LINES: &str = "50";

/// Follow the log file: print the tail of what is there, then whatever arrives.
///
/// `tail -F` rather than a follower of our own. It waits for a file that does not exist yet, which
/// is the first run on a machine before anything has been logged, and it reopens by name if the
/// file is replaced. Verified on macOS 25.5.0 against a path created a second after `tail` started:
/// the lines appear.
///
/// The child inherits this terminal's stdio and its process group, so Ctrl-C reaches it and ends
/// the follow. That is the whole reason `start` puts the daemon in a group of its own.
///
/// The log records `debug` whatever `LOG_LEVEL` says, and this prints it verbatim; narrowing it is
/// a grep over the text, not a filter this can apply.
pub(crate) fn logs() -> i32 {
    let path = crate::logging::log_path();
    println!("mercury: following {}", path.display());
    match Command::new(TAIL)
        .args(["-n", TAIL_LINES])
        .arg("-F")
        .arg(&path)
        .status()
    {
        Ok(status) => i32::from(!status.success()),
        Err(e) => {
            eprintln!("mercury: could not run {TAIL}: {e}");
            1
        }
    }
}
```

Verifying: `mercury logs` with no daemon running follows an empty or absent file and prints nothing; starting a daemon in another pane makes its lines appear. Ctrl-C returns to the shell.

## Change 7: `start`, and the bare `mercury`

```rust
/// How long `start` waits for a spawned daemon to take the lock.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a daemon is up, once `start` is done trying.
enum Started {
    /// One is running: this call started it, or found it already there.
    Running,
    /// None is running and this call could not start one.
    Failed,
}

/// Start the daemon unless one is already running.
///
/// The check before spawning is for the message, not for mutual exclusion. Two `start`s at the same
/// instant can both see [`Held::Free`] and both spawn, and the lock refuses one of the two daemons
/// exactly as it refuses a second `mercury daemon`; nothing here has to be atomic.
fn start() -> Started {
    match freddie_single_instance::holder(APP) {
        Ok(Held::By(pid)) => {
            println!("mercury is already running (pid {pid})");
            return Started::Running;
        }
        Ok(Held::Unnamed) => {
            println!("mercury is already running");
            return Started::Running;
        }
        Ok(Held::Free) => {}
        Err(e) => {
            eprintln!("mercury: {e}");
            return Started::Failed;
        }
    }
    match spawn_daemon() {
        Ok(pid) => {
            if wait_until_held() {
                println!("mercury started (pid {pid})");
                Started::Running
            } else {
                eprintln!(
                    "mercury: the daemon did not start; see {}",
                    crate::logging::log_path().display()
                );
                Started::Failed
            }
        }
        Err(e) => {
            eprintln!("mercury: could not start the daemon: {e}");
            Started::Failed
        }
    }
}

/// Spawn this same binary as `mercury daemon`, detached from this terminal.
///
/// `current_exe` rather than a configured path: the child is whatever binary the user invoked, so
/// `cargo run -p mercury` starts the debug build under `target/` and an installed binary starts
/// itself, with nothing to keep in step.
///
/// `process_group(0)` puts the child in a process group of its own, so Ctrl-C in this terminal goes
/// to the foreground group (this process, and the `tail` that follows it) and not to the daemon.
/// Without it, quitting the log stream would quit mercury. It also keeps the daemon off the SIGHUP
/// the kernel sends the session's foreground group when the terminal closes. Verified on the pinned
/// 1.96.0 toolchain: `process_group` is a safe method on `std::os::unix::process::CommandExt`, so
/// this costs no `unsafe` and no `libc`.
///
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer has no terminal to
/// write to and the log file is the record; an inherited stdout would interleave the daemon's
/// output with the follower's.
fn spawn_daemon() -> io::Result<u32> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let exe = std::env::current_exe()?;
    // The environment is inherited, so `LOG_LEVEL=debug mercury` reaches the daemon, where it
    // governs a terminal layer writing to /dev/null. The file records `debug` regardless.
    let child = Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(child.id())
}

/// Poll until something holds the lock, up to [`START_TIMEOUT`]. `true` when one does.
///
/// Taking the lock is the readiness signal, and the daemon takes it first thing, before it measures
/// the screens, shows an icon, or grabs the keyboard. So this returning `true` says the process is
/// alive and is the one mercury, not that the keyboard is grabbed: a daemon refused Accessibility
/// fails a moment later, and says so in the log this points at.
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

/// The bare `mercury`: make sure a daemon is up, then watch what it does.
pub(crate) fn start_and_follow() -> i32 {
    match start() {
        Started::Running => logs(),
        Started::Failed => 1,
    }
}

/// `mercury start`: the same, without staying to watch.
pub(crate) fn start_only() -> i32 {
    match start() {
        Started::Running => 0,
        Started::Failed => 1,
    }
}

/// `mercury restart`: replace the running daemon with a fresh one.
///
/// The two halves are already sequenced by the lock. `stop_daemon` returns only once the lock is
/// free, which is the same condition `start` needs to find, so the new daemon never races the old
/// one's shutdown and reports "already running" against the process it just killed.
///
/// A daemon that would not stop means no start is attempted: the old process still owns the tap,
/// and spawning a second one that the lock immediately refuses would say nothing useful.
///
/// Starting from cold is a restart with an empty first half, rather than an error, so a script that
/// restarts on a rebuild does not have to know whether anything was up.
pub(crate) fn restart() -> i32 {
    match stop_daemon() {
        Stopped::Was(pid) => println!("mercury stopped (pid {pid})"),
        Stopped::NotRunning => println!("mercury was not running"),
        Stopped::Failed => return 1,
    }
    start_only()
}
```

`parse` changes its default:

```diff
     let Some(verb) = args.next() else {
-        return Ok(Command::Daemon);
+        return Ok(Command::StartAndFollow);
     };
```

with `Command::StartAndFollow`, `Command::Start`, and `Command::Restart` added, `USAGE` reaching the full text at the top of this doc, and the `no_verb_runs_the_daemon` test becoming:

```rust
#[test]
fn no_verb_starts_and_follows() {
    assert!(matches!(parse_args(&[]), Ok(Command::StartAndFollow)));
}

#[test]
fn the_restart_verb_restarts() {
    assert!(matches!(parse_args(&["restart"]), Ok(Command::Restart)));
}
```

Verifying, from a shell with no mercury running:

- `mercury` prints "mercury started (pid N)" and then follows the log. Ctrl-C returns to the shell; `mercury status` still reports pid N, and the keyboard still remaps.
- `mercury` again prints "mercury is already running (pid N)" and follows.
- Closing the terminal entirely leaves the daemon up, by `mercury status` from a new one.
- `mercury restart` prints "mercury stopped (pid N)" then "mercury started (pid M)" with `M` a different pid, and `mercury status` reports `M`. The log shows the old daemon's `SIGTERM: quitting` and `kill: exiting` before the new one's `initial state`.
- `mercury restart` with nothing running prints "mercury was not running" and starts one.
- Editing a binding, `cargo build -p mercury && ./target/debug/mercury restart`, and the new binding is live without any window having been touched.
- `mercury stop` stops it, and `mercury status` exits 1.
- `tccutil reset Accessibility` and then `mercury`: the daemon takes the lock, the follower shows the `could not intercept the keyboard` line, and the process exits.

## What `launch-at-login.md` inherits

Its plist runs `mercury daemon`, and `launchctl bootout` becomes a clean shutdown through change 1. That doc's open question about a hand-started mercury fighting the loaded agent is already answered by the lock: `mercury start` finds the agent's daemon holding it and starts nothing.
