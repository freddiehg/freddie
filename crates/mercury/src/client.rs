//! The client verbs: everything `mercury` does that is not being the daemon.
//!
//! None of these takes the single-instance lock. They probe it to find the daemon, and the daemon
//! is the only process that holds it.

use std::fmt;
use std::io;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};
use tracing::{Level, debug, info, warn};

use crate::cli::{LogsArgs, RestartArgs, StopArgs};
use crate::logging::{self, Terminal};

/// The app name the lock is keyed to. The daemon acquires this; the clients probe it.
pub(crate) const APP: &str = "mercury";

/// How often [`find_daemon`] re-probes a lock whose holder has not named itself yet.
const POLL: Duration = Duration::from_millis(10);

/// How long `stop` waits for the daemon to release the lock before reporting that it has not.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `start` waits for a spawned daemon to take the lock.
const START_TIMEOUT: Duration = Duration::from_secs(5);

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
    Ignored(SignalIgnored),
}

/// A daemon that outlasted the signal sent to it, and which signal that was.
///
/// The remedy differs: a daemon that outlasted SIGTERM can still be destroyed, and one that
/// outlasted SIGKILL cannot be destroyed by anything.
struct SignalIgnored {
    pid: Pid,
    signal: Signal,
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
            // No verb named: `stop` and `restart` both reach this, and `--force` is on whichever
            // one was typed.
            Self::Ignored(SignalIgnored { pid, signal }) => match signal {
                Signal::Terminate => {
                    write!(f, "pid {pid} still holds the lock; --force destroys it")
                }
                // SIGKILL cannot be caught, but its delivery waits for an uninterruptible system
                // call to return. Nothing else to suggest: the process dies when that returns.
                Signal::Kill => write!(
                    f,
                    "pid {pid} outlasted SIGKILL, so it is stuck in a system call and will go when that returns"
                ),
            },
        }
    }
}

/// Ask the running daemon to go, and wait for it to let go of the lock.
///
/// `Ok(None)` is the daemon that was not there: nothing to stop is not a failure, so a teardown
/// script that does not know the state is not wrong to call this.
///
/// `stop` and `restart` both need the outcome and word it differently, so this reports facts and
/// says nothing to the terminal. Its records are `debug!`, which is the rule for a client verb:
/// `info!` is the answer and reaches stdout, `warn!` and above are the problem and reach stderr,
/// and `debug!` is what it did along the way, which only the file keeps. Narrating here at `info!`
/// would print three lines where the verb has one thing to say.
fn stop_daemon(signal: Signal) -> Result<Option<Pid>, Failure> {
    let pid = match find_daemon() {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Ok(None),
        Ok(Target::Anonymous) => {
            debug!("the lock is held by a holder that recorded no pid");
            return Err(Failure::Anonymous);
        }
        Err(error) => {
            debug!(%error, "could not read the lock");
            return Err(Failure::Unreadable(error));
        }
    };
    // Before the signal, so the wait cannot miss a daemon that exits between the two.
    let freed = watch_for_free();
    debug!(daemon = %pid, ?signal, "signalling the daemon");
    if let Err(error) = signal_pid(pid, signal) {
        debug!(daemon = %pid, %error, "could not signal the daemon");
        return Err(Failure::Unsignalable(SignalFailure { pid, error }));
    }
    if matches!(freed.recv_timeout(STOP_TIMEOUT), Ok(Ok(()))) {
        debug!(daemon = %pid, "the daemon released the lock");
        Ok(Some(pid))
    } else {
        debug!(daemon = %pid, ?signal, timeout = ?STOP_TIMEOUT, "the daemon still holds the lock");
        Err(Failure::Ignored(SignalIgnored { pid, signal }))
    }
}

/// `mercury stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop(args: &StopArgs) -> i32 {
    logging::init(&Terminal::Client);
    // Before looking for anything, so a stop that found nothing running still leaves a record that
    // somebody asked. `debug!` rather than `info!`: it is an action, not the verb's answer, and the
    // answer should be the only thing the terminal shows.
    debug!(force = args.force, "stop requested");
    let signal = if args.force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(signal) {
        Ok(Some(pid)) => {
            info!("mercury stopped (pid {pid})");
            0
        }
        Ok(None) => {
            info!("mercury is not running");
            0
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            1
        }
    }
}

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
/// Reports facts and says nothing to the terminal, as [`stop_daemon`] does, because `start` and
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
/// Polled rather than waited on, unlike [`watch_for_free`]: flock reports a release, so waiting
/// for a daemon to go is edge-triggered, and there is no way to wait on another process taking a
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

/// Say what [`ensure_started`] found, and report the exit code for it.
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

/// `mercury start`, and the bare `mercury`: make sure a daemon is up, and do not stay to watch.
pub(crate) fn start() -> i32 {
    logging::init(&Terminal::Client);
    report(ensure_started())
}

/// `mercury restart`: replace the running daemon with a fresh one.
///
/// The two halves are already sequenced by the lock. [`stop_daemon`] returns only once the lock is
/// free, which is the same condition [`ensure_started`] needs to find, so the new daemon never
/// races the old one's shutdown and reports "already running" against the process it just replaced.
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

/// Report whether the daemon is running, and which process it is.
///
/// Exits 1 when nothing is running, so `mercury status && ...` reads the way a shell expects. That
/// is the opposite of `stop`, which exits 0 having found nothing to stop, and both are deliberate:
/// this verb answers a question, so its exit code is the answer, while `stop` states a goal that a
/// stopped mercury already satisfies.
pub(crate) fn status() -> i32 {
    logging::init(&Terminal::Client);
    match freddie_single_instance::holder(APP) {
        Ok(Held::Free) => {
            info!("mercury is not running");
            1
        }
        Ok(Held::By(pid)) => {
            info!("mercury is running (pid {pid})");
            0
        }
        // The window between a daemon taking the lock and writing its pid into it. Something is
        // running, and that is the question this verb was asked, so it answers yes without the pid
        // rather than waiting for one.
        //
        // `stop` treats the same state as a failure, because a signal needs a pid and there is
        // none. Neither is wrong: they are asking the lock different questions.
        Ok(Held::Unnamed) => {
            info!("mercury is running (it has just started and has not recorded its pid)");
            0
        }
        Err(e) => {
            warn!("mercury: {e}");
            1
        }
    }
}

/// Where `tail` lives. Absolute, so `PATH` cannot point this at something else.
const TAIL: &str = "/usr/bin/tail";

/// How much of the existing log to show before following.
const TAIL_LINES: &str = "50";

/// The level of a record, read out of the line the `fmt` layer wrote.
///
/// A stamped record is `pid=N TIMESTAMP LEVEL target: message`, so the level is the third
/// whitespace-separated token; splitting on whitespace also drops the padding the formatter puts
/// in front of the shorter names. `None` for a line that is not a record.
///
/// Reading the text is what filtering a formatted log costs. A machine format would remove the
/// coupling and break what the file is for: `CLAUDE.md` sends a person, or an agent, to read it
/// directly.
fn record_level(line: &str) -> Option<Level> {
    line.split_whitespace().nth(2)?.parse().ok()
}

/// Dim, for the part of a record that is the same on every line.
const DIM: &str = "\x1b[2m";

/// Back to the terminal's own colours.
const RESET: &str = "\x1b[0m";

/// The colour `fmt` would have given a level, had the file been written in colour.
fn level_color(level: Level) -> &'static str {
    match level.as_str() {
        "ERROR" => "\x1b[31m",
        "WARN" => "\x1b[33m",
        "INFO" => "\x1b[32m",
        "DEBUG" => "\x1b[34m",
        "TRACE" => "\x1b[35m",
        _ => "",
    }
}

/// Show one record, colouring it the way the daemon's own terminal would have.
///
/// The file is written with ANSI off, because `CLAUDE.md` sends a person or an agent to read it
/// with `cat` and `grep`, and escapes in the file would defeat both. Colour is added here instead,
/// where the format is known: a record is `pid=N TIMESTAMP LEVEL target: message`, so splitting on
/// the level's own name divides the prefix that is the same every line from the rest.
///
/// A line with no level, or a stdout that is not a terminal, is written through unchanged.
fn show(out: &mut impl Write, line: &str, level: Option<Level>, color: bool) -> io::Result<()> {
    let split = level
        .filter(|_| color)
        .and_then(|level| Some((level, line.split_once(level.as_str())?)));
    match split {
        Some((level, (head, rest))) => writeln!(
            out,
            "{DIM}{head}{RESET}{}{}{RESET}{rest}",
            level_color(level),
            level.as_str()
        ),
        None => writeln!(out, "{line}"),
    }
}

/// Follow the log file: show the tail of what is there, then whatever arrives.
///
/// `tail -F` rather than a follower of our own. It waits for a file that does not exist yet, which
/// is the first run on a machine before anything has been logged, and it reopens by name if the
/// file is replaced.
///
/// Its stdout is piped rather than inherited, so each line can be dropped or shown. Its stderr and
/// its process group are inherited, so Ctrl-C reaches it and ends the follow. That is the whole
/// reason `mercury-start.md` puts the daemon in a group of its own.
///
/// Lines are written straight to stdout rather than traced: they are already records, out of the
/// file this is following, and tracing them would put them back into it.
pub(crate) fn logs(args: &LogsArgs) -> i32 {
    // `init` returns where it put the log, which is the file to follow.
    let path = logging::init(&Terminal::Client);
    info!("mercury: following {}", path.display());

    let mut tail = match Command::new(TAIL)
        .args(["-n", TAIL_LINES])
        .arg("-F")
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("mercury: could not run {TAIL}: {e}");
            return 1;
        }
    };

    let Some(stdout) = tail.stdout.take() else {
        warn!("mercury: {TAIL} gave no stdout to read");
        return 1;
    };
    // Asked once: a pipeline gets the file's plain text, a terminal gets colour.
    let color = std::io::stdout().is_terminal();
    let mut out = std::io::stdout().lock();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let level = record_level(&line);
        // A line with no level is not a record: a wrapped message, or something that reached the
        // file without going through the formatter. Shown rather than dropped, because hiding what
        // we cannot classify is how a log loses the one line that mattered.
        if level.is_none_or(|level| level <= args.level) {
            // A closed stdout is the pipeline this was feeding going away, which ends the follow
            // rather than being worth a word about. Noticed on the next record rather than at
            // once, because that is when the write happens: `mercury logs | head -3` waits for
            // the daemon to say one more thing, exactly as `tail -f | head -3` does.
            if show(&mut out, &line, level, color).is_err() {
                break;
            }
        }
    }
    match tail.wait() {
        Ok(status) => i32::from(!status.success()),
        Err(e) => {
            warn!("mercury: {TAIL} could not be waited on: {e}");
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

#[cfg(test)]
mod tests {
    use super::{record_level, show};
    use tracing::Level;

    const RECORD: &str =
        "pid=34322 2026-07-19T13:11:19.717128Z  INFO mercury::daemon: SIGTERM: quitting";

    fn shown(line: &str, color: bool) -> String {
        let mut out = Vec::new();
        show(&mut out, line, record_level(line), color).expect("writing to a Vec");
        String::from_utf8(out).expect("the record is utf8")
    }

    #[test]
    fn the_level_is_the_third_token() {
        assert_eq!(record_level(RECORD), Some(Level::INFO));
        assert_eq!(
            record_level("pid=1 2026-07-19T13:11:19.717128Z DEBUG a: b"),
            Some(Level::DEBUG)
        );
    }

    // Anything that did not come out of the formatter has no level, and is shown rather than
    // dropped.
    #[test]
    fn a_line_that_is_not_a_record_has_no_level() {
        assert_eq!(record_level("a stray line"), None);
        assert_eq!(record_level(""), None);
    }

    // A pipeline gets what the file holds, byte for byte.
    #[test]
    fn without_colour_a_record_is_passed_through() {
        assert_eq!(shown(RECORD, false), format!("{RECORD}\n"));
    }

    // The prefix is dimmed, the level takes its own colour, and the message is left alone.
    #[test]
    fn with_colour_the_level_is_painted() {
        assert_eq!(
            shown(RECORD, true),
            "\x1b[2mpid=34322 2026-07-19T13:11:19.717128Z  \x1b[0m\x1b[32mINFO\x1b[0m \
             mercury::daemon: SIGTERM: quitting\n"
        );
    }

    #[test]
    fn a_line_with_no_level_is_never_painted() {
        assert_eq!(shown("a stray line", true), "a stray line\n");
    }
}
