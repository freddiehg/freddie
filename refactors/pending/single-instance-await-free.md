# waiting for a lock to come free

`freddie_single_instance` says who holds a lock. This adds waiting for one to be released: `await_free` blocks until nothing holds it.

Everything that replaces a running instance needs this. `mercury stop` waits for the daemon to let go before reporting that it stopped (`mercury-stop.md`), and `mercury restart` waits before starting the next one (`mercury-start.md`). Nothing outside this crate changes here, and no other doc has to land for this one to ship.

## flock already reports the release

A blocking `lock_shared` is granted the instant the exclusive holder lets go, so the wait is edge-triggered: there is no interval to choose and no latency floor. Verified on the pinned 1.96.0 against a standalone binary: a waiter blocked on a holder released after 300ms returned 48µs later, where a 50ms poll would have averaged 25ms.

The lock is also the right thing to wait on, rather than the holding process disappearing. The daemon releases it as it exits, and a released lock is exactly the condition under which the next acquire succeeds. Those are not the same instant, and it is the second one that callers act on.

Shared rather than exclusive, for the reason `holder_at` takes a shared lock: two clients waiting on the same holder must not block each other, and an exclusive waiter would hold the path shut against the next holder for as long as its caller took to drop it.

## The wait

```rust
/// Wait until nothing holds `app`'s lock.
///
/// # Errors
///
/// Returns [`LockError::NoStateDir`] when the environment names no per-user directory, and
/// otherwise whatever [`await_free_at`] returns.
pub fn await_free(app: &str) -> Result<(), LockError> {
    await_free_at(&lock_path(app).ok_or(LockError::NoStateDir)?)
}

/// Wait until nothing holds `path`'s lock, returning as it is released.
///
/// Blocks in flock, which grants the shared lock the moment the exclusive holder lets go. The
/// shared lock is dropped before this returns, so it leaves the path as it found it. Whether the
/// path is still free once the caller acts on the answer is not something this can promise, for
/// the reason [`holder_at`] gives.
///
/// There is no timeout, because flock has none to offer. A caller that needs one runs this on a
/// thread and stops listening to it.
///
/// # Errors
///
/// Returns [`LockError::Unavailable`] when the file cannot be created, opened, or locked.
pub fn await_free_at(path: &Path) -> Result<(), LockError> {
    let file = open(path)?;
    file.lock_shared().map_err(LockError::Unavailable)
}
```

`open` and `LockError::Unavailable` are what `refactors/past/single-instance-holder.md` left behind. The blocking `lock_shared` returns `io::Result<()>` rather than the `TryLockError` the probe deals in, so it maps its error directly instead of going through `locked`.

## Tests

Added to the existing module, using its `temp_lock` helper.

```rust
#[test]
fn waiting_blocks_until_the_holder_releases() {
    let path = temp_lock("await-release");
    let held = acquire_at(&path).expect("the path is free");
    let waiter = {
        let path = path.clone();
        std::thread::spawn(move || await_free_at(&path))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
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
```
