//! One process at a time, per app.
//!
//! [`acquire`] takes an exclusive lock on a file under [`lock_path`]; the second
//! process to ask is refused. The lock belongs to the open file description, so the
//! kernel drops it when the holder dies, however it dies, and a crashed process
//! leaves nothing behind for the next one to clear.
//!
//! The lock is the only thing that means anything. Whether the file exists, and what it
//! contains, mean nothing on their own, which is what makes a leftover file from the
//! last run the normal case rather than a stale artifact to detect and clean up.
//!
//! The holder writes its pid into a sibling file so [`holder`] can say which process is
//! running. The lock stays on the lock file and never covers the pid, so reading the pid
//! never contends with the lock: a mandatory lock (Windows) refuses reads of the bytes it
//! covers, and a pid kept under the lock could not be read back while a holder held it.
//!
//! That pid is read only when the lock is refused, so a pid belonging to a process that
//! has since died is never reported: the lock is free, and the probe answers
//! [`Held::Free`] without opening the pid file. A pid here is an address for a process
//! already known to be alive, never the evidence that it is.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The per-user directory this platform keeps app state in, or `None` when the
/// environment does not say where that is.
///
/// Each is the platform's directory for state that persists across runs, deliberately
/// not its cache or runtime directory. Deleting a lock file out from under its holder
/// lets the next process lock a fresh inode at the same path, which is two live
/// processes and no mutual exclusion, and both of those directories are swept: macOS
/// prunes `$TMPDIR` through `dirhelper`, and the XDG spec permits removing anything in
/// `XDG_RUNTIME_DIR` that has gone six hours without access. A lock file is never
/// touched after it is opened, so it is exactly what such a sweep collects.
#[cfg(target_os = "macos")]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
}

/// `%LOCALAPPDATA%`, which is per-machine: a roaming profile must not sync one
/// machine's lock file onto another.
#[cfg(target_os = "windows")]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// `$XDG_STATE_HOME`, defaulting to `~/.local/state` as the XDG base directory
/// specification says it should.
#[cfg(all(unix, not(target_os = "macos")))]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
}

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("freddie_single_instance has no per-user state directory for this platform");

/// Where `app`'s lock file lives, or `None` when the environment does not name a
/// per-user directory to put it in.
///
/// The path is absolute or it is nothing: a relative path would resolve against the
/// current directory, so the same app started from two directories would lock two
/// different files and both copies would run.
#[must_use]
pub fn lock_path(app: &str) -> Option<PathBuf> {
    Some(state_dir()?.join(app).join(format!("{app}.lock")))
}

/// Where the holder of `lock` records its pid, a sibling of the lock file rather than the
/// lock file itself: a mandatory lock (Windows) refuses reads of the range it covers, so a
/// probe can only read the pid from a file the lock does not touch.
fn pid_path(lock: &Path) -> PathBuf {
    lock.with_extension("pid")
}

/// A held claim on being the only instance of an app. Holding it keeps every other
/// instance out; dropping it, or exiting by any route, lets the next one in.
#[derive(Debug)]
pub struct Instance {
    _file: File,
}

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
    ///
    /// This cannot outlive the acquire that opened it: [`acquire_at`] hands back an
    /// [`Instance`] only once the pid is recorded, and a failed write fails the acquire
    /// and releases the lock. A caller that wants a pid retries briefly rather than
    /// indefinitely.
    Unnamed,
}

/// The lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another instance holds it.
    AlreadyRunning(PathBuf),
    /// The environment names no per-user directory to keep the lock file in.
    NoStateDir,
    /// The lock file could not be created, opened, or locked.
    Unavailable(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning(path) => write!(
                f,
                "another instance is already running (it holds {})",
                path.display()
            ),
            Self::NoStateDir => {
                f.write_str("no per-user directory to keep the lock file in; is HOME set?")
            }
            Self::Unavailable(e) => write!(f, "could not take the single-instance lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Claim the lock for `app`, at its per-user path.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory,
/// and otherwise whatever [`acquire_at`] returns.
pub fn acquire(app: &str) -> Result<Instance, LockError> {
    acquire_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Open `path`, creating the parent directory if it is missing.
///
/// Read and write both, because a shared lock is meant to permit reads and some platforms
/// want the handle to carry the access the lock grants; the file's contents are never
/// touched through it either way.
///
/// `truncate(false)` because opening must not disturb whatever an earlier run left, and
/// nothing here writes to this file at all: it is a lock target and nothing more.
fn open(path: &Path) -> Result<File, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Unavailable)
}

/// Report what a `try_lock` family call said, naming `path` in the refusal.
fn locked(path: &Path, result: Result<(), std::fs::TryLockError>) -> Result<(), LockError> {
    match result {
        Ok(()) => Ok(()),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::AlreadyRunning(path.to_owned())),
        Err(std::fs::TryLockError::Error(e)) => Err(LockError::Unavailable(e)),
    }
}

/// Take `path`'s exclusive lock: the claim on being the only instance, held by the
/// daemon for as long as it runs.
fn lock_exclusive(path: &Path) -> Result<File, LockError> {
    let file = open(path)?;
    locked(path, file.try_lock())?;
    Ok(file)
}

/// Take `path`'s shared lock: the question [`holder_at`] asks, refused only by an
/// exclusive holder.
///
/// Shared rather than exclusive so that two probes do not refuse each other. An
/// exclusive probe would read the losing side's answer out of a file the last real run
/// left a pid in, and report a dead process as live.
fn lock_shared(path: &Path) -> Result<File, LockError> {
    let file = open(path)?;
    locked(path, file.try_lock_shared())?;
    Ok(file)
}

/// Write this process's pid into the file beside the lock, replacing whatever an earlier
/// run left there.
///
/// `truncate(true)` because the previous run's pid may be longer than this one's, and a
/// short number written over a long one leaves trailing digits that parse as a pid
/// belonging to nobody. The handle closes as this returns; nothing keeps the pid file open
/// and nothing locks it, so a probe reads it freely.
///
/// The parent directory already exists: [`acquire_at`] takes the lock first, and locking
/// created it.
fn record_pid(path: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(std::process::id().to_string().as_bytes())?;
    file.flush()
}

/// The pid the file at `path` names, or `None` when it holds nothing that reads as one.
///
/// `path` is the pid file, a sibling of the lock; see [`pid_path`]. Meaningful only while
/// the lock is held, which is the one condition [`holder_at`] reads it under.
fn read_pid(path: &Path) -> Option<Pid> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok().map(Pid)
}

/// Claim `path` for this process, or report that another process holds it.
///
/// `try_lock` rather than `lock`: a second instance is refused immediately instead of
/// blocking, so a caller that cannot run is told so rather than left waiting for a
/// process that may never exit.
///
/// An `Instance` means the lock is held and the pid is recorded, both or neither.
/// Failing to write the pid fails the acquire, rather than handing back a lock nobody
/// can address: the file is open and writable by the time we hold its lock, so a failure
/// here is the disk going away, and an instance that nothing can find by pid is not one
/// worth handing back.
///
/// # Errors
///
/// Returns [`LockError::AlreadyRunning`] when another process holds the lock, and
/// [`LockError::Unavailable`] when the file cannot be created, opened, locked, or
/// written.
pub fn acquire_at(path: &Path) -> Result<Instance, LockError> {
    let file = lock_exclusive(path)?;
    record_pid(&pid_path(path)).map_err(LockError::Unavailable)?;
    Ok(Instance { _file: file })
}

/// Who holds `app`'s lock right now.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory,
/// and otherwise whatever [`holder_at`] returns.
pub fn holder(app: &str) -> Result<Held, LockError> {
    holder_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Who holds `path` right now, found by trying to take a shared lock and reading the
/// file when that is refused.
///
/// Taking it is the proof that no exclusive holder had it, and the lock is released
/// again before this returns. So the answer describes the instant it was asked, and a
/// process may start or exit immediately afterwards. Callers act on it knowing that;
/// [`acquire`] remains the only thing that decides who runs.
///
/// # Errors
///
/// Returns [`LockError::Unavailable`] when the file cannot be created, opened, or
/// locked.
pub fn holder_at(path: &Path) -> Result<Held, LockError> {
    match lock_shared(path) {
        // Shared lock acquired means nothing holds exclusive; dropping the file frees the lock.
        Ok(_probe) => Ok(Held::Free),
        Err(LockError::AlreadyRunning(_)) => {
            Ok(read_pid(&pid_path(path)).map_or(Held::Unnamed, Held::By))
        }
        Err(e) => Err(e),
    }
}

/// Wait until nothing holds `app`'s lock.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory,
/// and otherwise whatever [`await_free_at`] returns.
pub fn await_free(app: &str) -> Result<(), LockError> {
    await_free_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Wait until nothing holds `path`'s lock, returning as it is released.
///
/// Blocks in flock, which grants the shared lock the moment the exclusive holder lets
/// go, so this notices a release without an interval to poll on. The shared lock is
/// dropped before this returns, so it leaves the path as it found it. Whether the path
/// is still free once the caller acts on the answer is not something this can promise,
/// for the reason [`holder_at`] gives.
///
/// Shared rather than exclusive, as in [`holder_at`]: several callers waiting on one
/// holder are all granted the moment it releases, rather than each taking the path and
/// handing it on, which is both what a waiter means and the shorter window in which
/// observers hold a path the next holder is trying to acquire.
///
/// There is no timeout, because flock has none to offer. A caller that needs one runs
/// this on a thread and stops listening to it.
///
/// # Errors
///
/// Returns [`LockError::Unavailable`] when the file cannot be created, opened, or
/// locked.
pub fn await_free_at(path: &Path) -> Result<(), LockError> {
    let file = open(path)?;
    // The blocking form returns `io::Result`, not the `TryLockError` `locked` reads, and
    // has no refusal to report: it returns when the lock is granted or not at all.
    file.lock_shared().map_err(LockError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::{
        Held, Instance, LockError, Pid, acquire_at, await_free_at, holder_at, lock_path, pid_path,
        state_dir,
    };
    use std::path::PathBuf;
    use std::time::Duration;

    // A path of this test's own. Both halves of the name are needed, for different
    // collisions: `name` keeps libtest's threads, which share a pid, off each other's
    // files, and the pid keeps two test binaries running at once (a watch loop against
    // the pre-commit hook) off each other's. No test shares a `name`, so none of this
    // needs `--test-threads=1`. Nothing here ever locks a real app's path.
    fn temp_lock(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "freddie-single-instance-{}-{name}.lock",
            std::process::id()
        ))
    }

    #[test]
    fn a_second_acquire_is_refused() {
        let path = temp_lock("second");
        let _held = acquire_at(&path).expect("the first acquire takes the lock");
        assert!(matches!(acquire_at(&path), Err(LockError::AlreadyRunning(p)) if p == path));
    }

    #[test]
    fn releasing_lets_the_next_one_in() {
        let path = temp_lock("release");
        let held = acquire_at(&path).expect("the first acquire takes the lock");
        drop(held);
        acquire_at(&path).expect("the lock is free once the holder drops");
    }

    #[test]
    fn separate_paths_do_not_contend() {
        let (a, b) = (temp_lock("sep-a"), temp_lock("sep-b"));
        let _first: Instance = acquire_at(&a).expect("a is free");
        let _second: Instance = acquire_at(&b).expect("b is free and unrelated to a");
    }

    // The lock must never land on a relative path: two copies started from two
    // directories would lock two files and both would run.
    #[test]
    fn the_lock_path_is_absolute_and_named_for_its_app() {
        let path = lock_path("mercury").expect("the test environment names a state directory");
        assert!(path.is_absolute());
        assert!(path.ends_with("mercury/mercury.lock"));
    }

    #[test]
    fn the_lock_sits_under_the_platform_state_directory() {
        let dir = state_dir().expect("the test environment names a state directory");
        assert!(lock_path("mercury").expect("as above").starts_with(&dir));
    }

    #[test]
    fn the_holder_is_named_by_pid() {
        let path = temp_lock("holder-pid");
        let _held = acquire_at(&path).expect("the path is free");
        assert_eq!(
            holder_at(&path).expect("probing"),
            Held::By(Pid(std::process::id()))
        );
    }

    #[test]
    fn an_unlocked_path_is_free() {
        let path = temp_lock("holder-free");
        assert_eq!(holder_at(&path).expect("probing"), Held::Free);
    }

    // The property the whole design rests on: a pid outlives its process in the file,
    // and is never reported once the lock behind it is gone.
    #[test]
    fn a_released_lock_is_free_though_its_pid_remains() {
        let path = temp_lock("holder-stale");
        let held = acquire_at(&path).expect("the path is free");
        drop(held);
        assert_eq!(holder_at(&path).expect("probing"), Held::Free);
        let left = std::fs::read_to_string(pid_path(&path)).expect("the pid outlives the lock");
        assert_eq!(left.trim(), std::process::id().to_string());
    }

    // A probe must not stamp itself into a file it only asked about, or every
    // `mercury status` would leave a dead pid behind for the next reader.
    #[test]
    fn probing_writes_nothing() {
        let path = temp_lock("holder-readonly");
        assert_eq!(holder_at(&path).expect("probing"), Held::Free);
        assert!(
            std::fs::read_to_string(&path)
                .expect("the probe created it")
                .is_empty()
        );
    }

    // Probes must not mistake each other for a daemon. Under an exclusive probe this
    // fails: one probe refuses the others, and they answer with the pid an earlier run
    // left in the file, reporting a dead process as live while nothing is running.
    #[test]
    fn probes_do_not_refuse_each_other() {
        let path = temp_lock("holder-concurrent");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(pid_path(&path), "4294967295").expect("a pid from an earlier run");
        std::thread::scope(|scope| {
            let probes: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| holder_at(&path).expect("probing")))
                .collect();
            for probe in probes {
                assert_eq!(probe.join().expect("the probing thread"), Held::Free);
            }
        });
    }

    // A probe must be refused by the daemon, which is the only reason the shared lock is
    // a lock at all.
    #[test]
    fn a_probe_is_refused_by_the_holder() {
        let path = temp_lock("holder-exclusive");
        let _held = acquire_at(&path).expect("the path is free");
        assert!(matches!(holder_at(&path).expect("probing"), Held::By(_)));
    }

    // A longer pid from a previous run must not leave a tail behind a shorter one.
    #[test]
    fn a_recorded_pid_replaces_the_whole_file() {
        let path = temp_lock("holder-truncate");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(pid_path(&path), "4294967295").expect("a longer pid from an earlier run");
        let _held = acquire_at(&path).expect("the path is free");
        let written = std::fs::read_to_string(pid_path(&path)).expect("reading it back");
        assert_eq!(written, std::process::id().to_string());
    }

    #[test]
    fn waiting_blocks_until_the_holder_releases() {
        let path = temp_lock("await-release");
        let held = acquire_at(&path).expect("the path is free");
        let waiter = std::thread::spawn(move || await_free_at(&path));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!waiter.is_finished(), "the lock is still held");
        drop(held);
        waiter
            .join()
            .expect("the waiting thread")
            .expect("the lock came free");
    }

    #[test]
    fn waiting_on_a_free_path_returns_at_once() {
        let path = temp_lock("await-free");
        await_free_at(&path).expect("nothing holds it");
    }

    // Every waiter on one holder is released, none left behind the others.
    #[test]
    fn every_waiter_is_released() {
        let path = temp_lock("await-many");
        let held = acquire_at(&path).expect("the path is free");
        let waiters: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || await_free_at(&path))
            })
            .collect();
        std::thread::sleep(Duration::from_millis(50));
        drop(held);
        for waiter in waiters {
            waiter
                .join()
                .expect("the waiting thread")
                .expect("the lock came free");
        }
    }

    // The wait leaves the path as it found it, or the next acquire would be refused by whoever
    // just finished waiting for it.
    #[test]
    fn waiting_leaves_the_path_free() {
        let path = temp_lock("await-releases");
        await_free_at(&path).expect("nothing holds it");
        acquire_at(&path).expect("the wait let go of what it took");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_that_is_application_support() {
        let path = lock_path("mercury").expect("the test environment sets HOME");
        assert!(path.ends_with("Library/Application Support/mercury/mercury.lock"));
    }
}
