# one mercury at a time

A second mercury turns every keypress into an unbounded event storm. Mercury takes an exclusive file lock at startup and refuses to run when another process holds it.

## The failure

Each mercury installs its own session tap and mints its own `tag` (`freddie_keyboard::sys::macos::new_tag`), and its callback passes an event only when `EVENT_SOURCE_USER_DATA == tag`. `EmitterState::post` builds a fresh `CGEvent` and stamps its own tag on it. So with A and B running:

```
physical key -> A swallows it, emits a copy tagged tag_A
tag_A copy   -> A passes (tag matches), B swallows it, emits a copy tagged tag_B
tag_B copy   -> B passes (tag matches), A swallows it, emits a copy tagged tag_A
...
```

Each hop dispatches through both models, mutating both layers, and writes an `INFO dispatch` line. Measured on 2026-07-18, from `~/Library/Logs/mercury/mercury.log` at 16:29:12 through 16:29:20 with two mercuries up: 118,000 lines in 8 seconds, 13,000-16,000 per second, ending only when one of the two was quit from the menu bar.

## The lock

`open(2)` with `O_EXLOCK | O_NONBLOCK`, through `OpenOptionsExt::custom_flags`, which is a safe call. Mercury keeps `unsafe_code = "forbid"`; no `libc::flock` and no raw fd handling.

The properties, verified on macOS 25.5.0 (Darwin) against a standalone binary:

- A second `open` from another process fails with `EWOULDBLOCK`, surfacing as `io::ErrorKind::WouldBlock`.
- A second `open` from the *same* process also fails, because the lock belongs to the open file description rather than the process. That is what lets the tests below run in one process.
- The kernel releases the lock when the holder dies by any means, `SIGKILL` included, so there is no stale lock and nothing to clean up. The zero-byte lock file stays on disk between runs and is reused.

## Where the lock file lives

`~/Library/Application Support/mercury/mercury.lock`, and the directory it sits in is a correctness constraint rather than a convention.

The lock belongs to the inode, not to the path. Delete the file while a mercury holds it and the holder keeps its lock, but the next mercury creates a fresh file at the same path and takes it without contest: two mercuries, and nothing in the log saying why. So the lock must live somewhere the system never prunes, which means:

- Not `$TMPDIR` (`/var/folders/<user>/T/`). `com.apple.bsd.dirhelper` runs `/usr/libexec/dirhelper` at load and daily at 03:35 with `CLEAN_FILES_OLDER_THAN_DAYS=3`. Mercury is a login agent that holds the lock for weeks and never touches the file after `open`, so its atime never advances; it is exactly what that job deletes.
- Not `~/Library/Caches` or `/var/folders/<user>/C/`, purgeable under disk pressure by design.
- Not `~/Library/Logs/mercury/`, next to the log file. `logging::log_dir` already builds that directory, but log directories are what people and tools prune, and pruning this one silently disables the guard.

Per-user rather than system-wide (`/var/run`), because two users logged in under fast user switching each have their own session and their own tap, so each legitimately gets a mercury.

## Change: `mercury` refuses to start when another mercury holds the lock

### `crates/mercury/Cargo.toml`

Before:

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
```

After:

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
# `O_EXLOCK` and `O_NONBLOCK` for the single-instance lock, as constants only: the open
# goes through `OpenOptionsExt::custom_flags`, which is safe.
libc = "0.2"
```

### `crates/mercury/src/instance.rs` (new)

```rust
//! One mercury at a time.
//!
//! Two mercuries ping-pong: each swallows the other's emissions and re-emits them
//! under its own tag, so one keypress becomes an unbounded storm. [`acquire`] takes
//! an exclusive lock on a file under [`lock_path`]; the second process to ask is
//! refused. The kernel drops the lock when the holder dies, however it dies, so a
//! crashed mercury leaves nothing behind for the next one to clear.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// The lock file, written under [`lock_dir`].
const LOCK_FILE: &str = "mercury.lock";

/// Where the lock file lives: the macOS per-user application-support directory, or
/// the current directory when `HOME` is unset. The same shape as `logging::log_dir`.
fn lock_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Application Support/mercury"),
    )
}

/// The path [`acquire`] locks for a real run.
pub fn lock_path() -> PathBuf {
    lock_dir().join(LOCK_FILE)
}

/// A held claim on being the only mercury. Dropping it, or exiting by any route,
/// releases the lock.
pub struct Instance {
    _file: File,
}

/// The lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another mercury holds it.
    AlreadyRunning(PathBuf),
    /// The lock file itself could not be created or opened.
    Unavailable(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning(path) => write!(
                f,
                "another mercury is already running (it holds {}); quit it from its menu-bar icon",
                path.display()
            ),
            Self::Unavailable(e) => write!(f, "could not take the single-instance lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Claim `path` for this process, or report who has it.
///
/// `O_EXLOCK | O_NONBLOCK` locks the file as it opens it and fails rather than waits,
/// so a second mercury is refused instead of hanging with the keyboard half-grabbed.
///
/// # Errors
///
/// Returns [`LockError::AlreadyRunning`] when another process holds the lock, and
/// [`LockError::Unavailable`] when the file cannot be created or opened at all.
pub fn acquire(path: &Path) -> Result<Instance, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .custom_flags(libc::O_EXLOCK | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => LockError::AlreadyRunning(path.to_owned()),
            _ => LockError::Unavailable(e),
        })?;
    Ok(Instance { _file: file })
}
```

### `crates/mercury/src/main.rs`

The lock comes after `logging::init`, so a refusal reaches the log file, and before everything that touches the machine: no window measurement, no status item, and above all no second `CGEventTap`.

Before:

```rust
mod logging;

fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
```

After:

```rust
mod instance;
mod logging;

fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // Before anything that touches the machine. Two mercuries swallow and re-emit each
    // other's keys forever, at tens of thousands of events a second, which wedges the
    // keyboard. The binding must outlive main (`let _instance`, never `let _`): dropping
    // it releases the lock, and `let _` would drop it here.
    let _instance = match instance::acquire(&instance::lock_path()) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("mercury: {e}");
            error!(error = %e, "another mercury holds the lock; not starting");
            return;
        }
    };

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
```

### Tests, in `instance.rs`

They lock paths under `std::env::temp_dir()` and never `lock_path()`, so running the suite cannot lock out a mercury that is up, and a mercury that is up cannot fail the suite. The name argument keeps two tests in the same process off the same file.

```rust
#[cfg(test)]
mod tests {
    use super::{Instance, LockError, acquire, lock_path};
    use std::path::PathBuf;

    // A path of this test's own, so the suite never touches the real lock. Both halves of
    // the name are needed, and for different collisions: `name` keeps libtest's threads,
    // which share a pid, off each other's files, and the pid keeps two test binaries running
    // at once (a watch loop against the pre-commit hook) off each other's. Tests never share
    // a `name`, so nothing here needs `--test-threads=1`.
    fn temp_lock(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mercury-{}-{name}.lock", std::process::id()))
    }

    #[test]
    fn a_second_acquire_is_refused() {
        let path = temp_lock("second");
        let _held = acquire(&path).expect("the first acquire takes the lock");
        assert!(matches!(
            acquire(&path),
            Err(LockError::AlreadyRunning(p)) if p == path
        ));
    }

    #[test]
    fn releasing_lets_the_next_one_in() {
        let path = temp_lock("release");
        let held = acquire(&path).expect("the first acquire takes the lock");
        drop(held);
        acquire(&path).expect("the lock is free once the holder drops");
    }

    #[test]
    fn separate_paths_do_not_contend() {
        let (a, b) = (temp_lock("sep-a"), temp_lock("sep-b"));
        let _first: Instance = acquire(&a).expect("a is free");
        let _second: Instance = acquire(&b).expect("b is free and unrelated to a");
    }

    #[test]
    fn the_lock_sits_beside_the_other_per_user_state() {
        let path = lock_path();
        assert!(path.ends_with("Library/Application Support/mercury/mercury.lock"));
    }
}
```

## Verifying it by hand

```
cargo build -p mercury
./target/debug/mercury &            # takes the lock
./target/debug/mercury              # prints the refusal and exits, keyboard untouched
kill -9 %1                          # the kernel drops the lock
./target/debug/mercury              # starts normally
```

The log file records the refusal at `ERROR`, so a launchd-started mercury losing the race is visible after the fact.

## Against launchd

`launch-at-login.md` has launchd refusing a second `bootstrap` of the same label. That covers agent-versus-agent only. The lock is what stops a hand-started `./target/debug/mercury` from fighting the loaded agent, which is the case that actually happens while developing.
