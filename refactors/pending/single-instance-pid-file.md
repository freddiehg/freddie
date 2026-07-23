# the pid lives beside the lock, not inside it

`freddie_single_instance` stores the holder's pid inside the file it locks, and a probe reads that pid back once the lock is refused. That read works only because Unix `flock` is advisory: a second handle reads bytes the lock covers. Windows locks are mandatory, so the same read is refused, and `holder_at` cannot name a running daemon.

This moves the pid into a sibling file that is never locked, on every platform. Reading it never lands on a locked byte range, so it works everywhere through one code path, and the crate stops resting on advisory-lock read-through, the platform behavior whose absence broke Windows.

## The Windows failure

`acquire_at` holds `File::try_lock` (an exclusive whole-file lock) for the life of the `Instance`. `read_pid` opens a second handle and reads the pid text out of that same file.

On Windows the lock is `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK` over the whole file, and it is enforced: a `ReadFile` on the locked range through any other handle fails with `ERROR_LOCK_VIOLATION` (os error 33), even from the same process. So `read_pid` returns `None` and `holder_at` reports `Held::Unnamed` for a daemon that is plainly running.

The three tests that read the pid while a holder is live fail on `windows-latest`, and pass on macOS and Linux, where the lock is advisory and the read goes through:

```
test tests::the_holder_is_named_by_pid ... FAILED      // got Unnamed, wanted By(pid)
test tests::a_probe_is_refused_by_the_holder ... FAILED // got Unnamed, wanted By(_)
test tests::a_recorded_pid_replaces_the_whole_file ... FAILED
                    // reading it back: os error 33, another process has locked the file
```

This is the whole of what keeps the `portable (windows-latest)` CI job red.

## What we build, on every platform

The lock file becomes a pure lock target: created, locked, never written, never read. The pid moves to `<app>.pid`, a sibling in the same directory. The holder writes it there after taking the lock; a refused probe reads it there. One code path, no `cfg`, so CI on any platform exercises the real logic rather than a per-platform arm that no other platform runs.

The invariant is untouched. The pid is read exactly when the lock is refused, so a pid left behind by a dead run is never reported: that run's lock is free, the probe answers `Held::Free` and never opens the pid file. The pid file is a leftover in the way the lock file already is, and means nothing until the lock says a live process is home. This keeps the pid from ever being read as evidence of liveness, which is the failure `refactors/past/single-instance.md` exists to avoid.

The public surface does not change. `acquire_at` and `holder_at` still take the lock path; the pid path is derived from it with `Path::with_extension`, so `freddie_cli`, which passes `instance.lock_file()` to both, needs no change. Ships as one commit: the holder writing the pid to the new file and the probe reading it from there are the two halves of one move, and either alone leaves the crate broken.

## The module doc

Before:

```rust
//! The holder writes its pid into the file so [`holder`] can say which process is
//! running. That pid is read only when the lock is refused, so a pid belonging to a
//! process that has since died is never reported: its file's lock is free, and the
//! probe answers [`Held::Free`] without reading. A pid here is an address for a process
//! already known to be alive, never the evidence that it is.
```

After:

```rust
//! The holder writes its pid into a sibling file so [`holder`] can say which process is
//! running. The lock stays on the lock file and never covers the pid, so reading the pid
//! never contends with the lock: a mandatory lock (Windows) refuses reads of the bytes it
//! covers, and a pid kept under the lock could not be read back while a holder held it.
//!
//! That pid is read only when the lock is refused, so a pid belonging to a process that
//! has since died is never reported: the lock is free, and the probe answers
//! [`Held::Free`] without opening the pid file. A pid here is an address for a process
//! already known to be alive, never the evidence that it is.
```

## The pid path, beside the lock

New, next to `lock_path`:

```rust
/// Where the holder of `lock` records its pid, a sibling of the lock file rather than the
/// lock file itself: a mandatory lock (Windows) refuses reads of the range it covers, so a
/// probe can only read the pid from a file the lock does not touch.
fn pid_path(lock: &Path) -> PathBuf {
    lock.with_extension("pid")
}
```

`lock_path("mercury")` is `<state>/mercury/mercury.lock`, so `pid_path` of it is `<state>/mercury/mercury.pid`. A test's `temp_lock` path ends in `.lock` too, so the same swap gives it a matching `.pid` sibling.

## Opening the lock file

The lock file is no longer read or written, only locked. Its `open` loses the reason it read `read(true)`; the comment is what changes, and the flags stay so the handle can still take either lock.

Before:

```rust
/// Open `path`, creating the parent directory if it is missing.
///
/// `read(true)` alongside the write: the pid is read back through this same open mode,
/// and a write-only handle cannot serve that.
///
/// `truncate(false)` is the point, not an oversight: opening must not disturb the file
/// before the lock is held, and the holder truncates deliberately in [`record_pid`]
/// once it is.
fn open(path: &Path) -> Result<File, LockError> {
```

After:

```rust
/// Open `path`, creating the parent directory if it is missing.
///
/// Read and write both, because a shared lock is meant to permit reads and some platforms
/// want the handle to carry the access the lock grants; the file's contents are never
/// touched through it either way.
///
/// `truncate(false)` because opening must not disturb whatever an earlier run left, and
/// nothing here writes to this file at all: it is a lock target and nothing more.
fn open(path: &Path) -> Result<File, LockError> {
```

The body is unchanged:

```rust
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
```

## Recording the pid

`record_pid` took the already-locked handle and rewrote it in place with `set_len` and `seek`. It now opens the sibling file fresh and truncates it, which is the same guard against a longer old pid leaving a tail, without the seek.

Before:

```rust
/// Write this process's pid over whatever the file held.
///
/// `set_len` before the write, because the previous run's pid may be longer than this
/// one's, and writing a short number over a long one leaves trailing digits that parse
/// as a pid belonging to nobody.
fn record_pid(mut file: &File) -> io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(std::process::id().to_string().as_bytes())?;
    file.flush()
}
```

After:

```rust
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
```

## Reading the pid

`read_pid` is unchanged in body; the path it is handed is now the pid file's.

Before:

```rust
/// The pid the file at `path` names, or `None` when it holds nothing that reads as one.
///
/// Meaningful only while the lock is held; see [`holder_at`].
fn read_pid(path: &Path) -> Option<Pid> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok().map(Pid)
}
```

After:

```rust
/// The pid the file at `path` names, or `None` when it holds nothing that reads as one.
///
/// `path` is the pid file, a sibling of the lock; see [`pid_path`]. Meaningful only while
/// the lock is held, which is the one condition [`holder_at`] reads it under.
fn read_pid(path: &Path) -> Option<Pid> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok().map(Pid)
}
```

## Acquiring and probing

`acquire_at` writes the pid to the sibling after taking the lock. `holder_at` reads it from the sibling when the lock is refused. Both derive the pid path from the lock path they are already given.

`acquire_at`, before:

```rust
pub fn acquire_at(path: &Path) -> Result<Instance, LockError> {
    let file = lock_exclusive(path)?;
    record_pid(&file).map_err(LockError::Unavailable)?;
    Ok(Instance { _file: file })
}
```

`acquire_at`, after:

```rust
pub fn acquire_at(path: &Path) -> Result<Instance, LockError> {
    let file = lock_exclusive(path)?;
    record_pid(&pid_path(path)).map_err(LockError::Unavailable)?;
    Ok(Instance { _file: file })
}
```

The doc comment's guarantee holds unchanged: an `Instance` means the lock is held and the pid is recorded, both or neither, because a failed `record_pid` fails the acquire and drops `file`, which releases the lock.

`holder_at`, before:

```rust
pub fn holder_at(path: &Path) -> Result<Held, LockError> {
    match lock_shared(path) {
        // Dropping the file here closes it, which releases the lock we just took.
        Ok(_probe) => Ok(Held::Free),
        Err(LockError::AlreadyRunning(_)) => Ok(read_pid(path).map_or(Held::Unnamed, Held::By)),
        Err(e) => Err(e),
    }
}
```

`holder_at`, after:

```rust
pub fn holder_at(path: &Path) -> Result<Held, LockError> {
    match lock_shared(path) {
        // Dropping the file here closes it, which releases the lock we just took.
        Ok(_probe) => Ok(Held::Free),
        Err(LockError::AlreadyRunning(_)) => {
            Ok(read_pid(&pid_path(path)).map_or(Held::Unnamed, Held::By))
        }
        Err(e) => Err(e),
    }
}
```

## Imports

`record_pid` no longer seeks, so `Seek` and `SeekFrom` drop out.

Before:

```rust
use std::io::{Read, Seek, SeekFrom, Write};
```

After:

```rust
use std::io::{Read, Write};
```

## Tests

Three tests read the pid where the holder wrote it. They move to the pid file, and the test module imports `pid_path` to name it.

The import, before:

```rust
    use super::{
        Held, Instance, LockError, Pid, acquire_at, await_free_at, holder_at, lock_path, state_dir,
    };
```

After:

```rust
    use super::{
        Held, Instance, LockError, Pid, acquire_at, await_free_at, holder_at, lock_path, pid_path,
        state_dir,
    };
```

`a_released_lock_is_free_though_its_pid_remains` reads the leftover pid from the pid file rather than the lock file.

Before:

```rust
    let left = std::fs::read_to_string(&path).expect("the file outlives the lock");
    assert_eq!(left.trim(), std::process::id().to_string());
```

After:

```rust
    let left = std::fs::read_to_string(pid_path(&path)).expect("the pid outlives the lock");
    assert_eq!(left.trim(), std::process::id().to_string());
```

`probes_do_not_refuse_each_other` seeds the earlier run's leftover pid in the pid file, which is where a real earlier run would have left it.

Before:

```rust
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(&path, "4294967295").expect("a pid from an earlier run");
```

After:

```rust
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(pid_path(&path), "4294967295").expect("a pid from an earlier run");
```

`a_recorded_pid_replaces_the_whole_file` seeds and reads the pid file, and its comment already names the property (a longer old pid must not leave a tail behind a shorter one), which `record_pid`'s `truncate(true)` keeps.

Before:

```rust
    let path = temp_lock("holder-truncate");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(&path, "4294967295").expect("a longer pid from an earlier run");
    let _held = acquire_at(&path).expect("the path is free");
    let written = std::fs::read_to_string(&path).expect("reading it back");
    assert_eq!(written, std::process::id().to_string());
```

After:

```rust
    let path = temp_lock("holder-truncate");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(pid_path(&path), "4294967295").expect("a longer pid from an earlier run");
    let _held = acquire_at(&path).expect("the path is free");
    let written = std::fs::read_to_string(pid_path(&path)).expect("reading it back");
    assert_eq!(written, std::process::id().to_string());
```

`the_holder_is_named_by_pid` and `a_probe_is_refused_by_the_holder` are unchanged in text: they read through `holder_at`, which now finds the pid in the sibling file, so they go green on Windows without editing. `probing_writes_nothing` reads the lock file and asserts it is empty, which stays true now that nothing writes to it. The rest of the module is untouched.

After the change, `cargo test -p freddie_single_instance` passes on `windows-latest`, which is what turns the `portable` CI job green, and the same code runs on macOS and Linux.
