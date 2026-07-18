# one mercury at a time

A second mercury turns every keypress into an unbounded event storm. `freddie_single_instance` takes an exclusive lock on a per-app file at startup, and mercury refuses to run when another process holds it. Any other binary in the family gets the same guarantee by calling the same two functions.

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

`File::try_lock`, stable since 1.89 and so available on the pinned 1.96 toolchain. No dependency, no `libc`, no `unsafe`, and `flock` on unix with `LockFileEx` on Windows rather than anything BSD-specific.

Opening and locking are two steps rather than one atomic `open(O_EXLOCK)`. Nothing races: two processes can both reach the `try_lock`, and exactly one of them gets `Ok(())` while the other gets `TryLockError::WouldBlock`.

The properties, verified on macOS 25.5.0 (Darwin) against a standalone binary:

- A second `try_lock` from another process returns `TryLockError::WouldBlock`.
- A second `try_lock` from the *same* process also refuses, because the lock belongs to the open file description rather than to the process. That is what lets the tests below run in one process, on libtest's threads, without `--test-threads=1`.
- The kernel releases the lock when the holder dies by any means, `SIGKILL` included, so there is no stale lock and nothing to clean up. The zero-byte lock file stays on disk between runs and is reused.

The file is a rendezvous name, not storage. Nothing is ever written to it or read from it, so its contents and its existence carry no meaning; only the lock does. `create(true)`, never `create_new(true)`: a leftover file from the last run is the normal case, and treating it as "already running" is the stale-artifact bug this design exists to avoid.

## Why there is no orphaned lock

The lock belongs to the open file description, so it dies when the last fd referencing it closes. Nothing persists that a later process would have to inspect, age out, or clear. Measured on this machine:

- The holder exits normally, or panics: the next `acquire` succeeds.
- `SIGTERM`, `SIGKILL`: the next `acquire` succeeds.
- `SIGSTOP`: the next `acquire` is refused, correctly, because that process still owns the event tap.
- The holder spawns a child and then exits: the next `acquire` succeeds; the child does not hold the lock.
- The machine crashes or loses power: the next `acquire` succeeds, the lock having been kernel memory.

`SIGTERM` matters because mercury installs no signal handler, so its `Drop` impls do not run: today's log has a run with no `kill: exiting` line for exactly that reason. The kernel closes the fd during exit regardless, so `Instance`'s drop is a nicety and correctness does not rest on it.

The child case is the one that could break. `fork` shares the open file description and `exec` keeps it unless the fd is `FD_CLOEXEC`, and mercury spawns `open` through `std::process::Command` on every foreground (`freddie_app_nav::foreground`). Rust's std opens files `O_CLOEXEC`, so the lock fd is dropped at `exec` in every child. Verified by spawning a 30-second child while holding the lock, exiting the parent without dropping the `File`, and re-acquiring successfully. A later move to a raw `libc::open`, or a `pre_exec` that preserves fds, would reintroduce the leak.

## Where the lock file lives

`~/Library/Application Support/<app>/<app>.lock`, and the directory it sits in is a correctness constraint rather than a convention.

The lock belongs to the inode, not to the path. Delete the file while a holder is up and the holder keeps its lock on the now-unlinked inode, but the next process creates a fresh file at that path and locks that one instead: two live processes, two unrelated locks, and nothing in the log saying why. So the lock must live somewhere the system never prunes, which means:

- Not `$TMPDIR` (`/var/folders/<user>/T/`). `com.apple.bsd.dirhelper` runs `/usr/libexec/dirhelper` at load and daily at 03:35 with `CLEAN_FILES_OLDER_THAN_DAYS=3`. Mercury is a login agent that holds the lock for weeks and never touches the file after opening it, so its atime never advances; it is exactly what that job deletes.
- Not `~/Library/Caches` or `/var/folders/<user>/C/`, purgeable under disk pressure by design.
- Not `~/Library/Logs/mercury/`, next to the log file. `logging::log_dir` already builds that directory, but log directories are what people and tools prune, and pruning this one silently disables the guard.

Per-user rather than system-wide (`/var/run`), because two users logged in under fast user switching each have their own session and their own tap, so each legitimately gets a mercury.

## Change 1: the `freddie_single_instance` crate

Its own crate rather than a module in mercury, so the next binary in the family gets the guarantee by calling `acquire(&lock_path("figaro"))`. It has no dependencies. The lock itself is cross-platform; the path convention in `lock_path` is macOS.

The error text names no menu bar and no mercury: the crate reports the fact, and each caller phrases its own advice.

### `Cargo.toml` (workspace root)

Before:

```toml
    "crates/freddie_menu_bar",
    "crates/freddie_windows",
    "crates/mercury",
]
```

After:

```toml
    "crates/freddie_menu_bar",
    "crates/freddie_windows",
    "crates/freddie_single_instance",
    "crates/mercury",
]
```

### `crates/freddie_single_instance/Cargo.toml` (new)

```toml
[package]
name = "freddie_single_instance"
description = "One process at a time, per app, through an exclusive lock on a per-user file."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true
```

### `crates/freddie_single_instance/src/lib.rs` (new)

Compiles clean under the workspace lints (`clippy::all`, `pedantic`, `nursery`, `cargo`, all denied) and its four tests pass.

```rust
//! One process at a time, per app.
//!
//! [`acquire`] takes an exclusive lock on a file under [`lock_path`]; the second
//! process to ask is refused. The lock belongs to the open file description, so the
//! kernel drops it when the holder dies, however it dies, and a crashed process
//! leaves nothing behind for the next one to clear. The file is a rendezvous name:
//! nothing is written to it or read from it, and its existence means nothing.
//!
//! The lock is cross-platform; [`lock_path`]'s convention is macOS.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Where `app`'s lock file lives: the macOS per-user application-support directory,
/// or the current directory when `HOME` is unset.
///
/// Deliberately not a cache or a temp directory. macOS prunes both, and deleting a
/// lock file out from under its holder lets the next process lock a fresh inode at
/// the same path, which is two live processes and no mutual exclusion.
#[must_use]
pub fn lock_path(app: &str) -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(app)
                .join(format!("{app}.lock"))
        },
    )
}

/// A held claim on being the only instance. Dropping it, or exiting by any route,
/// releases the lock.
#[derive(Debug)]
pub struct Instance {
    _file: File,
}

/// The lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another instance holds it.
    AlreadyRunning(PathBuf),
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
            Self::Unavailable(e) => write!(f, "could not take the single-instance lock: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Claim `path` for this process, or report that another process holds it.
///
/// `try_lock` rather than `lock`: a second instance is refused immediately instead of
/// blocking, which for mercury means it exits rather than sitting there with the
/// keyboard half-grabbed.
///
/// # Errors
///
/// Returns [`LockError::AlreadyRunning`] when another process holds the lock, and
/// [`LockError::Unavailable`] when the file cannot be created, opened, or locked.
pub fn acquire(path: &Path) -> Result<Instance, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
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
    use super::{Instance, LockError, acquire, lock_path};
    use std::path::PathBuf;

    // A path of this test's own. Both halves of the name are needed, for different
    // collisions: `name` keeps libtest's threads, which share a pid, off each other's
    // files, and the pid keeps two test binaries running at once (a watch loop against
    // the pre-commit hook) off each other's. No test shares a `name`, so none of this
    // needs `--test-threads=1`. Nothing here ever locks a real app's path.
    fn temp_lock(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fsi-{}-{name}.lock", std::process::id()))
    }

    #[test]
    fn a_second_acquire_is_refused() {
        let path = temp_lock("second");
        let _held = acquire(&path).expect("the first acquire takes the lock");
        assert!(matches!(acquire(&path), Err(LockError::AlreadyRunning(p)) if p == path));
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
        assert!(lock_path("mercury").ends_with("Library/Application Support/mercury/mercury.lock"));
    }
}
```

Of the four, two cover this crate's own logic: the `WouldBlock` to `AlreadyRunning` mapping, and the path shape (so a later tidy-up into `$TMPDIR` fails the build rather than silently disabling the guard). The other two pin kernel behavior the crate depends on but did not write.

## Change 2: mercury takes the lock before it touches the machine

### `crates/mercury/Cargo.toml`

Before:

```toml
freddie_menu_bar = { path = "../freddie_menu_bar", version = "0.0.1" }
freddie_windows = { path = "../freddie_windows", version = "0.0.1" }
```

After:

```toml
freddie_menu_bar = { path = "../freddie_menu_bar", version = "0.0.1" }
freddie_windows = { path = "../freddie_windows", version = "0.0.1" }
freddie_single_instance = { path = "../freddie_single_instance", version = "0.0.1" }
```

### `crates/mercury/src/main.rs`

The lock comes after `logging::init`, so a refusal reaches the log file, and before everything that touches the machine: no window measurement, no status item, and above all no second `CGEventTap`.

Before:

```rust
fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
```

After:

```rust
fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // Before anything that touches the machine. Two mercuries swallow and re-emit each
    // other's keys forever, at tens of thousands of events a second, which wedges the
    // keyboard. The binding must outlive main (`let _instance`, never `let _`): dropping
    // it releases the lock, and `let _` would drop it here.
    let _instance = match freddie_single_instance::acquire(&freddie_single_instance::lock_path(
        "mercury",
    )) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("mercury: {e}; quit the running one from its menu-bar icon");
            error!(error = %e, "another mercury holds the lock; not starting");
            return;
        }
    };

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
```

## Verifying change 2 by hand

By hand, and only by hand. Nothing automated ever starts mercury: a running mercury grabs the keyboard of the machine it is on, and if the lock is broken, two of them wedge that keyboard at tens of thousands of events a second rather than failing a test. The crate's tests exercise `acquire` as the file operation it is, against scratch paths, and stop there.

```
cargo build -p mercury
./target/debug/mercury &            # takes the lock
./target/debug/mercury              # prints the refusal and exits, keyboard untouched
kill -9 %1                          # the kernel drops the lock
./target/debug/mercury              # starts normally
```

This is the only check that reaches the wiring, and the wiring is where the plausible bugs are: `let _` instead of `let _instance` releases the lock immediately and silently disables the guard, and putting the call after `intercept` lets a second instance grab the keyboard before it refuses. Both pass every test in change 1.

The log file records the refusal at `ERROR`, so a launchd-started mercury losing the race is visible after the fact.

## Against launchd

`launch-at-login.md` has launchd refusing a second `bootstrap` of the same label. That covers agent-versus-agent only. The lock is what stops a hand-started `./target/debug/mercury` from fighting the loaded agent, which is the case that actually happens while developing.
