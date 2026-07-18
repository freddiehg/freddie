# the lock names its holder

`freddie_single_instance` says whether an app is running. This adds which process it is: the holder writes its pid into the file it has locked, and a new `holder` reports it.

Everything that wants to talk to a running instance needs this first. `mercury stop` signals the pid (`mercury-stop.md`), and `mercury status` prints it (`mercury-status-and-logs.md`). Nothing outside this crate changes here, and no other doc has to land for this one to ship.

## A pid that cannot go stale

The lock is the liveness signal and the contents are only an address. The pid is read exactly when the lock is refused, so a pid belonging to a process that has since died is never reported: that file's lock is free, and the probe answers `Held::Free` without reading. A pid here is an address for a process already known to be alive, never the evidence that it is alive.

That is what keeps this clear of the stale-pid-file failure `refactors/past/single-instance.md` was written to avoid. Verified on the pinned 1.96.0 against a standalone binary: with the lock held the probe reads `By(Pid(73919))`; after the holder drops, the probe reads `Free` while the file still contains `73919`.

There is a window between taking the lock and writing the pid in which the file is empty. `Held::Unnamed` is that window, rather than something a caller has to infer from an empty string.

## The module doc

Before:

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

## What a probe finds

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

## Locking and recording split apart

`acquire_at`'s body becomes three pieces, so that a probe can take the lock without writing to the file.

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
    // open mode, and a write-only handle cannot serve that.
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
/// and writable by the time we hold its lock, so a failure here is the disk going away, and an
/// instance that nothing can find by pid is not one worth handing back.
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

`Instance` keeps its `_file` name. The field is never read, because the lock lives on the open file description rather than on anything we call, and the workspace denies `unused`, so naming it `file` fails the build.

## The probe

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
/// returns. So the answer describes the instant it was asked, and a process may start or exit
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

## Tests

Added to the existing module, using its `temp_lock` helper, which keys each path to both a name and this process's pid so the suite needs no `--test-threads=1` and never touches a real app's lock.

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
