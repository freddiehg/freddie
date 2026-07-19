//! The client verbs: everything `mercury` does that is not being the daemon.
//!
//! None of these takes the single-instance lock. They probe it to find the daemon, and the daemon
//! is the only process that holds it.

use std::fmt;
use std::io;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};
use tracing::{info, warn};

use crate::cli::StopArgs;
use crate::logging;

/// The app name the lock is keyed to. The daemon acquires this; the clients probe it.
pub(crate) const APP: &str = "mercury";

/// What a client verb shows on the terminal through tracing: nothing but errors.
///
/// The file layer records `debug` regardless, so the run is in the log either way. A client's
/// terminal output is its printed result, and a timestamped `INFO` line above it would be noise in
/// front of the thing the user actually asked for.
const CLIENT_LOG_LEVEL: &str = "error";

/// How often [`find_daemon`] re-probes a lock whose holder has not named itself yet.
const POLL: Duration = Duration::from_millis(10);

/// How long `stop` waits for the daemon to release the lock before reporting that it has not.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait out [`Held::Unnamed`], the window between a daemon locking the file and
/// writing its pid into it.
///
/// Ten polls. The window is a `set_len` and a `write_all` on an already-open file, so it closes in
/// microseconds, and a holder still anonymous after this is one no longer wait will help: either
/// its pid write failed, which fails its acquire and releases the lock, or it has been stopped
/// mid-acquire and will not finish at all. Failing fast reports that; waiting only delays it.
const PID_TIMEOUT: Duration = Duration::from_millis(100);

/// What a client found when it went looking for the daemon.
enum Target {
    /// Nothing holds the lock.
    NotRunning,
    /// The daemon, ready to be signalled.
    Running(Pid),
    /// Something holds the lock and never recorded a pid, so there is nothing to signal.
    Anonymous,
}

/// The signal `stop` sends, and what each one costs.
#[derive(Clone, Copy, Debug)]
enum Signal {
    /// SIGTERM. The daemon routes it into the event channel and leaves the way the menu bar's Quit
    /// does, opening the modifiers it swallowed on the way out.
    Terminate,
    /// SIGKILL. The kernel destroys the process, so no destructor runs, the keyboard grab is torn
    /// down rather than released, and a swallowed modifier stays down in the app underneath. The
    /// only out for a daemon whose worker is blocked in an effect, which SIGTERM cannot reach.
    Kill,
}

impl Signal {
    /// How `/bin/kill` spells it.
    const fn flag(self) -> &'static str {
        match self {
            Self::Terminate => "-TERM",
            Self::Kill => "-KILL",
        }
    }
}

/// Why a stop did not happen.
///
/// Separate variants because the remedies differ: `--force` answers [`Failure::Ignored`] and
/// nothing else. There is no pid to destroy in the other three.
enum Failure {
    /// The lock could not be read, so nothing is known about what holds it.
    Unreadable(LockError),
    /// Something holds the lock and recorded no pid, so there is nothing to signal.
    Anonymous,
    /// The signal could not be sent to the pid the lock named.
    Unsignalable(SignalFailure),
    /// The daemon was signalled and still holds the lock.
    Ignored(Pid),
}

/// A signal that could not be sent, and to whom.
struct SignalFailure {
    pid: Pid,
    error: io::Error,
}

/// The terminal wording for each, without the `mercury: ` a caller puts in front.
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "{e}"),
            Self::Anonymous => f.write_str(
                "something holds the lock but recorded no pid; it is starting or shutting down",
            ),
            Self::Unsignalable(SignalFailure { pid, error }) => {
                write!(f, "could not signal pid {pid}: {error}")
            }
            Self::Ignored(pid) => write!(
                f,
                "pid {pid} still holds the lock; `mercury stop --force` destroys it"
            ),
        }
    }
}

/// Ask the running daemon to go, and wait for it to let go of the lock.
///
/// `Ok(None)` is the daemon that was not there: nothing to stop is not a failure, so a teardown
/// script that does not know the state is not wrong to call this.
///
/// `stop` and `restart` both need the outcome and word it differently, so this reports facts and
/// prints nothing. It traces, because the log records where a thing happened rather than who is
/// being spoken to, but the terminal line is the caller's.
fn stop_daemon(signal: Signal) -> Result<Option<Pid>, Failure> {
    let pid = match find_daemon() {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Ok(None),
        Ok(Target::Anonymous) => {
            warn!("the lock is held by a holder that recorded no pid");
            return Err(Failure::Anonymous);
        }
        Err(error) => {
            warn!(%error, "could not read the lock");
            return Err(Failure::Unreadable(error));
        }
    };
    // Before the signal, so the wait cannot miss a daemon that exits between the two.
    let freed = watch_for_free();
    info!(daemon = %pid, ?signal, "signalling the daemon");
    if let Err(error) = signal_pid(pid, signal) {
        warn!(daemon = %pid, %error, "could not signal the daemon");
        return Err(Failure::Unsignalable(SignalFailure { pid, error }));
    }
    if matches!(freed.recv_timeout(STOP_TIMEOUT), Ok(Ok(()))) {
        info!(daemon = %pid, "the daemon released the lock");
        Ok(Some(pid))
    } else {
        warn!(daemon = %pid, timeout = ?STOP_TIMEOUT, "the daemon still holds the lock");
        Err(Failure::Ignored(pid))
    }
}

/// `mercury stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop(args: &StopArgs) -> i32 {
    logging::init(CLIENT_LOG_LEVEL);
    let signal = if args.force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(signal) {
        Ok(Some(pid)) => {
            println!("mercury stopped (pid {pid})");
            0
        }
        Ok(None) => {
            println!("mercury is not running");
            0
        }
        Err(failure) => {
            eprintln!("mercury: {failure}");
            1
        }
    }
}

/// Find the daemon, waiting out the window in which it holds the lock without having named itself.
///
/// That window is a `set_len` and a `write_all` on an already-open file, so it closes in
/// microseconds. A holder that stays anonymous past [`PID_TIMEOUT`] is one whose pid write failed,
/// which `acquire_at` treats as a failure to start, so its lock is about to be released anyway.
///
/// Polled rather than waited on, unlike [`watch_for_free`]: the lock is held throughout this
/// window, so flock has nothing to report and there is no edge to wait for.
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

/// Start waiting for the lock to come free, and hand back the channel it reports on.
///
/// `await_free` blocks in flock with no timeout of its own, so it runs on a thread and the caller
/// stops listening once its own deadline passes. Abandoning that thread costs nothing: `stop`
/// exits moments later and the process teardown takes it along.
fn watch_for_free() -> mpsc::Receiver<Result<(), LockError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(freddie_single_instance::await_free(APP));
    });
    rx
}

/// Send `signal` to `pid`.
///
/// A subprocess rather than a `kill(2)` binding, because the workspace forbids `unsafe` and every
/// binding for it is an unsafe extern call. The same trade `freddie_app_nav` makes by
/// foregrounding an app through `open`. An absolute path, so `PATH` cannot point this at something
/// else.
fn signal_pid(pid: Pid, signal: Signal) -> io::Result<()> {
    let status = Command::new("/bin/kill")
        .arg(signal.flag())
        .arg(pid.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("/bin/kill exited with {status}")))
    }
}
