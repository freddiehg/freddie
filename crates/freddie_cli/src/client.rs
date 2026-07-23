//! The client verbs: everything a binary does that is not being the daemon.
//!
//! None of these takes the single-instance lock. They probe it to find the daemon, and the daemon
//! is the only process that holds it.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use freddie_single_instance::{Held, LockError, Pid};
use tracing::{Level, debug, info, warn};

use crate::{App, DAEMON_VERB, Instance, TypedArgs};

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
    #[cfg(unix)]
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
    /// Something holds the lock and recorded no pid, so there is nothing to signal. Carries the
    /// lock path, so the message can name the two files to remove if the holder is stale.
    Anonymous(PathBuf),
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

/// The terminal wording for each, without the name a caller puts in front.
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "{e}"),
            Self::Anonymous(lock) => write!(
                f,
                "something holds the lock but recorded no pid; it is starting or shutting down. \
                 If it persists, a live process holds the lock without having named itself: find \
                 it with `lsof {}` and stop it",
                lock.display(),
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
fn stop_daemon(instance: &Instance, signal: Signal) -> Result<Option<Pid>, Failure> {
    let pid = match find_daemon(instance) {
        Ok(Target::Running(pid)) => pid,
        Ok(Target::NotRunning) => return Ok(None),
        Ok(Target::Anonymous) => {
            debug!("the lock is held by a holder that recorded no pid");
            return Err(Failure::Anonymous(instance.lock_file().to_owned()));
        }
        Err(error) => {
            debug!(%error, "could not read the lock");
            return Err(Failure::Unreadable(error));
        }
    };
    // Before the signal, so the wait cannot miss a daemon that exits between the two.
    let freed = watch_for_free(instance);
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

/// `stop`.
///
/// Exits 0 when there was nothing to stop, so calling this twice, or in a teardown script that
/// does not know the state, is not an error.
pub(crate) fn stop(instance: &Instance, force: bool) -> ExitCode {
    // Before looking for anything, so a stop that found nothing running still leaves a record that
    // somebody asked. `debug!` rather than `info!`: it is an action, not the verb's answer, and the
    // answer should be the only thing the terminal shows.
    debug!(force, "stop requested");
    let signal = if force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(instance, signal) {
        Ok(Some(pid)) => {
            info!("{} stopped (pid {pid})", instance.display_name());
            ExitCode::SUCCESS
        }
        Ok(None) => {
            info!("{} is not running", instance.display_name());
            ExitCode::SUCCESS
        }
        Err(failure) => {
            warn!("{}: {failure}", instance.display_name());
            ExitCode::FAILURE
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

/// The terminal wording for each, without the name a caller puts in front.
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
/// exactly as it refuses a second `daemon`; nothing here has to be atomic.
///
/// Reports facts and says nothing to the terminal, as [`stop_daemon`] does, because `start` and
/// `restart` word the outcome differently.
fn ensure_started<TApp: App>(
    instance: &Instance,
    typed: TypedArgs<'_>,
) -> Result<Running, NotStarted> {
    match freddie_single_instance::holder_at(instance.lock_file()) {
        Ok(Held::By(pid)) => return Ok(Running::Adopted(pid)),
        Ok(Held::Unnamed) => return Ok(Running::AdoptedUnnamed),
        Ok(Held::Free) => {}
        Err(error) => {
            debug!(%error, "could not read the lock");
            return Err(NotStarted::Unreadable(error));
        }
    }
    let pid = spawn_daemon::<TApp>(typed).map_err(|error| {
        debug!(%error, "could not spawn the daemon");
        NotStarted::Unspawnable(error)
    })?;
    debug!(daemon = %pid, "spawned the daemon");
    if wait_until_held(instance) {
        Ok(Running::Started(pid))
    } else {
        debug!(daemon = %pid, timeout = ?START_TIMEOUT, "the daemon never took the lock");
        Err(NotStarted::Silent(pid))
    }
}

/// Spawn this same binary as its own hidden `daemon` verb, detached from this terminal.
///
/// All three stdio streams go to /dev/null. The daemon's terminal tracing layer then has nowhere
/// to write, which is why `--log-level` is not passed through: it governs a terminal this child
/// does not have. The log file records `debug` regardless, and `logs` reads that.
fn spawn_daemon<TApp: App>(typed: TypedArgs<'_>) -> io::Result<Pid> {
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg(DAEMON_VERB)
        .args(typed.argv::<TApp>())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = detach(&mut command).spawn()?;
    Ok(Pid(child.id()))
}

/// Put the child where a terminal cannot reach it, so it outlives the one that spawned it.
#[cfg(unix)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a ctrl-c in the spawning terminal does not reach it.
    command.process_group(0)
}

#[cfg(windows)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    /// No console at all, which is what the three null stdio streams already say.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    /// No console control event reaches it.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
}

/// Poll until something holds the lock, up to [`START_TIMEOUT`]. `true` when one does.
///
/// Polled rather than waited on, unlike [`watch_for_free`]: flock reports a release, so waiting
/// for a daemon to go is edge-triggered, and there is no way to wait on another process taking a
/// lock. This is the one direction that has no edge.
///
/// Taking the lock is the readiness signal, and the daemon takes it first thing, before it
/// measures the screens, shows an icon, or grabs the keyboard. So this returning `true` says the
/// process is alive and is the one daemon, not that it finished starting: one that is refused
/// Accessibility fails a moment later and says so in the log.
fn wait_until_held(instance: &Instance) -> bool {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if !matches!(
            freddie_single_instance::holder_at(instance.lock_file()),
            Ok(Held::Free)
        ) {
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
fn report(instance: &Instance, running: Result<Running, NotStarted>) -> ExitCode {
    match running {
        Ok(Running::Started(pid)) => {
            info!("{} started (pid {pid})", instance.display_name());
            ExitCode::SUCCESS
        }
        Ok(Running::Adopted(pid)) => {
            info!("{} is already running (pid {pid})", instance.display_name());
            ExitCode::SUCCESS
        }
        Ok(Running::AdoptedUnnamed) => {
            info!(
                "{} is already running (it has not recorded its pid yet)",
                instance.display_name()
            );
            ExitCode::SUCCESS
        }
        Err(failure) => {
            warn!("{}: {failure}", instance.display_name());
            ExitCode::FAILURE
        }
    }
}

/// `start`, and the bare binary: make sure a daemon is up, and do not stay to watch.
pub(crate) fn start<TApp: App>(instance: &Instance, typed: TypedArgs<'_>) -> ExitCode {
    report(instance, ensure_started::<TApp>(instance, typed))
}

/// `restart`: replace the running daemon with a fresh one.
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
pub(crate) fn restart<TApp: App>(
    instance: &Instance,
    force: bool,
    typed: TypedArgs<'_>,
) -> ExitCode {
    let signal = if force {
        Signal::Kill
    } else {
        Signal::Terminate
    };
    match stop_daemon(instance, signal) {
        Ok(Some(pid)) => info!("{} stopped (pid {pid})", instance.display_name()),
        Ok(None) => debug!("nothing was running to stop"),
        Err(failure) => {
            warn!("{}: not restarting: {failure}", instance.display_name());
            return ExitCode::FAILURE;
        }
    }
    report(instance, ensure_started::<TApp>(instance, typed))
}

/// Report whether the daemon is running, and which process it is.
///
/// Exits nonzero when nothing is running, so `status && ...` reads the way a shell expects. That
/// is the opposite of `stop`, which exits 0 having found nothing to stop, and both are deliberate:
/// this verb answers a question, so its exit code is the answer, while `stop` states a goal that a
/// stopped daemon already satisfies.
pub(crate) fn status(instance: &Instance) -> ExitCode {
    match freddie_single_instance::holder_at(instance.lock_file()) {
        Ok(Held::Free) => {
            info!("{} is not running", instance.display_name());
            ExitCode::FAILURE
        }
        Ok(Held::By(pid)) => {
            info!("{} is running (pid {pid})", instance.display_name());
            ExitCode::SUCCESS
        }
        // The window between a daemon taking the lock and writing its pid into it. Something is
        // running, and that is the question this verb was asked, so it answers yes without the pid
        // rather than waiting for one.
        //
        // `stop` treats the same state as a failure, because a signal needs a pid and there is
        // none. Neither is wrong: they are asking the lock different questions.
        Ok(Held::Unnamed) => {
            info!(
                "{} is running (it has just started and has not recorded its pid)",
                instance.display_name()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            warn!("{}: {e}", instance.display_name());
            ExitCode::FAILURE
        }
    }
}

/// How many of the file's existing lines to show before following.
const BACKLOG_LINES: usize = 50;

/// How long to wait before looking for more, once a read has come up empty.
///
/// A poll, and the exception the "never poll" rule allows: no platform reports a regular file
/// growing through a readiness primitive. `epoll` and `kqueue` both call a regular file always
/// ready and return zero bytes, and `tail -F` polls for the same reason.
const IDLE: Duration = Duration::from_millis(200);

/// One record out of the log file.
///
/// `fields` is a map rather than a struct, because its keys are whatever the call site passed:
/// `message` is always there and the rest are the record's own. It is `#[serde(flatten)]` because
/// the layer writes those keys at the top level, so this collects everything that is not the
/// envelope, in the order the record holds them.
#[derive(serde::Deserialize)]
struct Record {
    pid: u32,
    timestamp: String,
    level: String,
    target: String,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

/// The fields `logs` leaves out unless asked for them.
///
/// The state is the whole model rendered with `Debug`, which is most of a dispatch record and is
/// read when something is being debugged and not before.
const VERBOSE_FIELDS: &[&str] = &["state"];

/// What `logs` renders, and how much of each record.
#[derive(Clone, Copy)]
pub(crate) struct LogsView {
    /// The least severe records to show.
    pub(crate) least: Level,
    /// Put the state field back on dispatch records.
    pub(crate) include_state: bool,
    /// Emit each record as the raw JSON it is stored as, for `jq`.
    pub(crate) json: bool,
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
/// with `grep` and `jq`, and escapes in the file would defeat both. Colour is added here instead.
///
/// The state is left out unless `view` asks for it: it is the whole model under `Debug`, most of
/// the line, and read while something is being debugged and not before.
fn show(out: &mut impl Write, record: &Record, view: &LogsView, color: bool) -> io::Result<()> {
    let (dim, reset, level_color) = if color {
        (
            DIM,
            RESET,
            level_color(record.level.parse().unwrap_or(Level::INFO)),
        )
    } else {
        ("", "", "")
    };
    write!(
        out,
        "{dim}{} pid={} {}{reset} {level_color}{}{reset}",
        record.timestamp, record.pid, record.target, record.level
    )?;
    if let Some(message) = record.fields.get("message") {
        write!(out, " {}", as_text(message))?;
    }
    for (key, value) in &record.fields {
        if key == "message" || (!view.include_state && VERBOSE_FIELDS.contains(&key.as_str())) {
            continue;
        }
        write!(out, " {dim}{key}={reset}{}", as_text(value))?;
    }
    writeln!(out)
}

/// A field as it reads: a string without its quotes, anything else as the JSON it is.
fn as_text(value: &serde_json::Value) -> Cow<'_, str> {
    match value {
        serde_json::Value::String(s) => Cow::Borrowed(s),
        other => Cow::Owned(other.to_string()),
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
/// reason `refactors/past/mercury-start.md` puts the daemon in a group of its own.
///
/// Lines are written straight to stdout rather than traced: they are already records, out of the
/// file this is following, and tracing them would put them back into it.
pub(crate) fn logs(instance: &Instance, view: LogsView) -> ExitCode {
    let path = instance.log_file();
    info!("{}: following {}", instance.display_name(), path.display());

    // Asked once: a pipeline gets the file's plain text, a terminal gets colour.
    let color = std::io::stdout().is_terminal();
    let mut out = std::io::stdout().lock();

    match follow(&path, &view, color, &mut out) {
        // The follow ends only when stdout closes, which is the pipeline this was feeding going
        // away: `logs | head -3` got its lines and left. That is a clean finish, not a failure.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            warn!(
                "{}: could not read {}: {e}",
                instance.display_name(),
                path.display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Show `path`'s last [`BACKLOG_LINES`] records, then whatever is appended to it, filtered to
/// `least` and above. Returns when `out` closes; errors only on a failure to read the file.
///
/// The file is opened once and never reopened: `tracing_appender::rolling::never` holds one file
/// for the life of the daemon and never rotates it, so there is no new inode to follow onto. A
/// reader sees appended bytes once they are written, and a line without a trailing newline is a
/// record the writer has not finished, held until the rest arrives.
fn follow(path: &Path, view: &LogsView, color: bool, out: &mut impl Write) -> io::Result<()> {
    let mut reader = BufReader::new(File::open(path)?);

    // The backlog, kept in a ring so the whole file is never held: a daemon appends to its log for
    // as long as it runs, and there is no bound on that.
    let mut backlog: VecDeque<String> = VecDeque::with_capacity(BACKLOG_LINES);
    let mut line = String::new();
    while reader.read_line(&mut line)? != 0 {
        if line.ends_with('\n') {
            if backlog.len() == BACKLOG_LINES {
                backlog.pop_front();
            }
            backlog.push_back(std::mem::take(&mut line));
        }
        // A trailing line with no newline is a partial record at EOF, left in `line` to finish as
        // the daemon writes the rest below.
    }
    for record in &backlog {
        if show_record(out, record, view, color).is_break() {
            return Ok(());
        }
    }

    // Then whatever arrives, a record at a time, waiting when the file has not grown.
    loop {
        if reader.read_line(&mut line)? != 0 && line.ends_with('\n') {
            if show_record(out, &line, view, color).is_break() {
                return Ok(());
            }
            line.clear();
        } else {
            std::thread::sleep(IDLE);
        }
    }
}

/// Write one record to `out`, filtered to `view.least` and shown as `view` asks. `Break` when
/// `out` has closed.
///
/// A line that is not a record is shown as it stands: a file written by an older daemon, or
/// something that reached the file without going through the formatter. Hiding what cannot be
/// classified is how a log loses the one line that mattered, so a record whose level does not parse
/// is shown on the same reasoning.
fn show_record(out: &mut impl Write, line: &str, view: &LogsView, color: bool) -> ControlFlow<()> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let Ok(record) = serde_json::from_str::<Record>(line) else {
        return broke(writeln!(out, "{line}").is_err());
    };
    if record
        .level
        .parse::<Level>()
        .is_ok_and(|level| level > view.least)
    {
        return ControlFlow::Continue(());
    }
    if view.json {
        broke(writeln!(out, "{line}").is_err())
    } else {
        broke(show(out, &record, view, color).is_err())
    }
}

/// `Break` on a write that failed, `Continue` otherwise: a closed `out` is the pipeline this was
/// feeding going away, which ends the follow rather than being an error worth a word.
const fn broke(failed: bool) -> ControlFlow<()> {
    if failed {
        ControlFlow::Break(())
    } else {
        ControlFlow::Continue(())
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
fn find_daemon(instance: &Instance) -> Result<Target, LockError> {
    let deadline = Instant::now() + PID_TIMEOUT;
    loop {
        match freddie_single_instance::holder_at(instance.lock_file())? {
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
fn watch_for_free(instance: &Instance) -> mpsc::Receiver<Result<(), LockError>> {
    let lock = instance.lock_file().to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(freddie_single_instance::await_free_at(&lock));
    });
    rx
}

/// Send `signal` to `pid`.
///
/// A subprocess rather than a `kill(2)` binding, because the workspace forbids `unsafe` and every
/// binding for it is an unsafe extern call. The same trade `freddie_app_nav` makes by
/// foregrounding an app through `open`. An absolute path, so `PATH` cannot point this at something
/// else.
#[cfg(unix)]
fn signal_pid(pid: Pid, signal: Signal) -> io::Result<()> {
    ran(Command::new("/bin/kill")
        .arg(signal.flag())
        .arg(pid.to_string()))
}

#[cfg(windows)]
fn signal_pid(pid: Pid, signal: Signal) -> io::Result<()> {
    match signal {
        // `taskkill /F` terminates the process, which is SIGKILL. `taskkill` without it posts
        // WM_CLOSE, which a daemon with no window never sees, so there is no gentle taskkill.
        Signal::Kill => ran(Command::new("taskkill")
            .args(["/F", "/PID"])
            .arg(pid.to_string())),
        Signal::Terminate => Err(io::Error::other(
            "graceful stop is not yet available on Windows; use --force",
        )),
    }
}

/// Run `command` to completion, turning a non-zero exit into an error.
fn ran(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} exited with {status}",
            command.get_program().to_string_lossy()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{LogsView, Record, show};
    use tracing::Level;

    const DISPATCH: &str = r#"{"pid":1,"timestamp":"2026-07-21T09:14:02.114Z","level":"INFO","message":"dispatch","event":"Key(KeyR)","effects":"[]","state":"Mercury { .. }","target":"mercury::daemon"}"#;

    fn view(include_state: bool) -> LogsView {
        LogsView {
            least: Level::DEBUG,
            include_state,
            json: false,
        }
    }

    fn shown(line: &str, view: &LogsView, color: bool) -> String {
        let record: Record = serde_json::from_str(line).expect("a record");
        let mut out = Vec::new();
        show(&mut out, &record, view, color).expect("writing to a Vec");
        String::from_utf8(out).expect("the record is utf8")
    }

    #[test]
    fn the_state_is_left_out_unless_asked_for() {
        let without = shown(DISPATCH, &view(false), false);
        assert!(without.contains("event=Key(KeyR)"));
        assert!(!without.contains("state="));

        let with = shown(DISPATCH, &view(true), false);
        assert!(with.contains("state=Mercury { .. }"));
    }

    // A dispatch record reads as its parts: the prefix, the message, then each field in the order
    // it was logged. `message` is pulled out first, `event` comes before `effects` because that is
    // how the daemon records them, and `state` is left out.
    #[test]
    fn a_record_reads_as_its_parts() {
        let line = shown(DISPATCH, &view(false), false);
        assert_eq!(
            line.trim_end(),
            "2026-07-21T09:14:02.114Z pid=1 mercury::daemon INFO dispatch event=Key(KeyR) effects=[]"
        );
    }

    // Anything that did not come out of the formatter is not a record, and `show_record` writes it
    // through rather than dropping it.
    #[test]
    fn a_line_that_is_not_a_record_does_not_parse() {
        assert!(
            serde_json::from_str::<Record>("Boot-out failed: 36: Operation now in progress")
                .is_err()
        );
    }
}
