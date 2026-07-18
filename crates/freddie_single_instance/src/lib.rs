//! One process at a time, per app.
//!
//! [`acquire`] takes an exclusive lock on a file under [`lock_path`]; the second
//! process to ask is refused. The lock belongs to the open file description, so the
//! kernel drops it when the holder dies, however it dies, and a crashed process
//! leaves nothing behind for the next one to clear.
//!
//! The file is a rendezvous name rather than storage. Nothing is written to it or read
//! from it, so neither its contents nor its existence mean anything; only the lock
//! does. That is what makes a leftover file from the last run the normal case rather
//! than a stale artifact to detect and clean up.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
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

/// A held claim on being the only instance of an app. Holding it keeps every other
/// instance out; dropping it, or exiting by any route, lets the next one in.
#[derive(Debug)]
pub struct Instance {
    _file: File,
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

/// Claim `path` for this process, or report that another process holds it.
///
/// `try_lock` rather than `lock`: a second instance is refused immediately instead of
/// blocking, so a caller that cannot run is told so rather than left waiting for a
/// process that may never exit.
///
/// # Errors
///
/// Returns [`LockError::AlreadyRunning`] when another process holds the lock, and
/// [`LockError::Unavailable`] when the file cannot be created, opened, or locked.
pub fn acquire_at(path: &Path) -> Result<Instance, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
    }
    // `truncate(false)` is the point, not an oversight: the file is a rendezvous name and
    // never holds anything, so there is nothing to shorten, and a lock must not disturb
    // whatever a previous version of the app may have left in it.
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Unavailable)?;
    match file.try_lock() {
        Ok(()) => Ok(Instance { _file: file }),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::AlreadyRunning(path.to_owned())),
        Err(std::fs::TryLockError::Error(e)) => Err(LockError::Unavailable(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Instance, LockError, acquire_at, lock_path, state_dir};
    use std::path::PathBuf;

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

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_that_is_application_support() {
        let path = lock_path("mercury").expect("the test environment sets HOME");
        assert!(path.ends_with("Library/Application Support/mercury/mercury.lock"));
    }
}
