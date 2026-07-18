# one mercury at a time

A second mercury turns every keypress into an unbounded event storm. `freddie_single_instance` takes an exclusive lock on a per-app file at startup, and mercury refuses to run when another process holds it. Any other binary in the family gets the same guarantee from `freddie_single_instance::acquire("its-name")`.

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

`File::try_lock`, stable since 1.89 and so available on the pinned 1.96 toolchain. No dependency, no `libc`, no `unsafe`, and `flock` on unix with `LockFileEx` on Windows rather than anything BSD-specific. `try_lock` rather than `lock`, so a second mercury is refused immediately instead of blocking behind a process that may never exit.

Opening and locking are two steps rather than one atomic `open(O_EXLOCK)`. Nothing races: two processes can both reach the `try_lock`, and exactly one gets `Ok(())` while the other gets `TryLockError::WouldBlock`.

The file is a rendezvous name, not storage. Nothing is written to it or read from it, so neither its contents nor its existence carry meaning; only the lock does. Hence `create(true)` with `truncate(false)`, never `create_new(true)`: a leftover file from the last run is the normal case, and treating its presence as "already running" is the stale-artifact bug this design exists to avoid.

## Why there is no orphaned lock

The lock belongs to the open file description, so it dies when the last fd referencing it closes. Nothing persists that a later process would have to inspect, age out, or clear. Measured on macOS 25.5.0 against a standalone binary:

- The holder exits normally, or panics: the next `acquire` succeeds.
- `SIGTERM`, `SIGKILL`: the next `acquire` succeeds.
- `SIGSTOP`: the next `acquire` is refused, correctly, because that process still owns the event tap.
- The holder spawns a child and then exits: the next `acquire` succeeds; the child does not hold the lock.
- The machine crashes or loses power: the next `acquire` succeeds, the lock having been kernel memory.

`SIGTERM` matters because mercury installs no signal handler, so its `Drop` impls do not run: the log has a run with no `kill: exiting` line for exactly that reason. The kernel closes the fd during exit regardless, so `Instance`'s drop is a nicety and correctness does not rest on it.

The child case is the one that could break. `fork` shares the open file description and `exec` keeps it unless the fd is `FD_CLOEXEC`, and mercury spawns `open` through `std::process::Command` on every foreground (`freddie_app_nav::foreground`). Rust's std opens files `O_CLOEXEC`, so the lock fd is dropped at `exec` in every child. Verified by spawning a 30-second child while holding the lock, exiting the parent without dropping the `File`, and re-acquiring successfully. A later move to a raw `libc::open`, or a `pre_exec` that preserves fds, would reintroduce the leak.

## Where the lock file lives

`state_dir` is per platform, and each branch names that platform's directory for state that persists across runs:

- macOS: `$HOME/Library/Application Support`
- Windows: `%LOCALAPPDATA%`, which is per-machine, so a roaming profile cannot sync one machine's lock file onto another
- Other unix: `$XDG_STATE_HOME`, defaulting to `$HOME/.local/state`

Anything neither unix nor Windows is a `compile_error!` rather than a guessed path.

Deliberately not a cache or runtime directory, and the reason is a correctness constraint rather than convention. The lock belongs to the inode, not the path. Delete the file while a holder is up and the holder keeps its lock on the now-unlinked inode, but the next process creates a fresh file at that path and locks that one instead: two live processes, two unrelated locks, and nothing in the log saying why. Both of the tempting directories are swept — macOS prunes `$TMPDIR` through `com.apple.bsd.dirhelper` (at load, and daily at 03:35, with `CLEAN_FILES_OLDER_THAN_DAYS=3`), and the XDG spec permits removing anything in `XDG_RUNTIME_DIR` that has gone six hours without access. A lock file is never touched after it is opened, so its atime never advances and it is exactly what such a sweep collects.

`lock_path` returns `Option<PathBuf>` for the same class of reason: a missing `HOME` becomes `LockError::NoStateDir` rather than a cwd-relative path, which would let `cd /a && mercury` and `cd /b && mercury` lock different inodes and both start.

Per-user rather than system-wide (`/var/run`), because two users logged in under fast user switching each have their own session and their own tap, so each legitimately gets a mercury.

## What shipped

`crates/freddie_single_instance` (commit `a7b5d2c`), no dependencies:

```rust
pub fn lock_path(app: &str) -> Option<PathBuf>;
pub fn acquire(app: &str) -> Result<Instance, LockError>;      // the per-user path
pub fn acquire_at(path: &Path) -> Result<Instance, LockError>; // any path; what the tests use

pub struct Instance { _file: File }

pub enum LockError {
    AlreadyRunning(PathBuf),
    NoStateDir,
    Unavailable(io::Error),
}
```

Its own crate rather than a module in mercury, so the next binary in the family gets the guarantee by calling `acquire("figaro")`. The error text names no menu bar and no mercury: the crate reports the fact, and each caller phrases its own advice.

Six tests. Two cover this crate's own logic — the `WouldBlock` to `AlreadyRunning` mapping, and the path being absolute, app-named, and under the platform's state directory, so a later tidy-up into `$TMPDIR` fails the build rather than silently disabling the guard. The rest pin kernel behavior the crate depends on but did not write. They lock scratch paths under `std::env::temp_dir`, never a real app's path, so the suite cannot lock out a running mercury and a running mercury cannot fail the suite. Each test's path carries both a name and the pid: the name keeps libtest's threads, which share a pid, off each other's files, and the pid keeps two concurrent test binaries off each other's. No `--test-threads=1` needed.

Clippy clean under the workspace lints on all three targets. The Windows and Linux branches are genuinely type-checked, confirmed by planting an undeclared type in the Windows branch and watching `cargo check --target x86_64-pc-windows-msvc` fail while the macOS build stayed green.

`crates/mercury/src/main.rs` (commit `d83c1b1`) takes the lock above `freddie_windows::init`, the status item, and `intercept`, so a refused instance measures no screens, shows no icon, and never installs a second tap:

```rust
let _instance = match freddie_single_instance::acquire("mercury") {
    Ok(instance) => instance,
    Err(e) => {
        eprintln!("mercury: {e}; quit the running one from its menu-bar icon");
        error!(error = %e, "another mercury holds the lock; not starting");
        return;
    }
};
```

`let _instance`, never `let _`, which would drop the guard at the end of that statement and release the lock immediately. Verified by hand, the only check that reaches this wiring: `cargo run -p mercury` twice, the second refusing. The lock is keyed to the path, not the binary, so `cargo run`, `./target/debug/mercury`, and a launchd agent all contend for the same file across rebuilds.

## Against launchd

`launch-at-login.md` has launchd refusing a second `bootstrap` of the same label. That covers agent-versus-agent only. The lock is what stops a hand-started `cargo run -p mercury` from fighting the loaded agent, which is the case that actually happens while developing.
