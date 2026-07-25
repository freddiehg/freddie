# a posted event drains what posting autoreleases

`CGEventPost` autoreleases two CoreFoundation objects per call. `Emitter::post` runs on the worker thread, which is a plain thread with no autorelease pool, so nothing ever drains them and they are leaked for the life of the run.

Measured on the live daemon, 2000 keys injected into a freshly started mercury:

```
before: footprint=16.60M  malloc_nodes=27574
after:  footprint=17.70M  malloc_nodes=31592
delta:  footprint=+1.10M  nodes=+4018        over 2000 keys
```

`heap` names them. Two classes move and nothing else does:

- `CFData`: 1572 to 3643, so +2071 for 2000 keys, 57.6 bytes each
- `CFData (Bytes Storage)`: 1273 to 3343, so +2070, 516.6 bytes each

That is two allocations and about 574 bytes per posted key. Confirmed again over a longer window of ordinary use: 8193 posts added 15,919 allocations (1.94 per post) and 4.40MB (563 bytes per post).

The pool is what is missing, not a release. Isolated, one long-lived private source, 500 events per case:

```
create event only, never posted              nodes=   +0 (+0.00/ev)     +0 B/ev
create event + CGEventPost                   nodes=+1001 (+2.00/ev)   +642 B/ev
CGEventPost inside an autorelease pool       nodes=   +0 (+0.00/ev)     +0 B/ev
```

Creating an event allocates nothing that outlives it, so the post is the only call that needs draining. The two objects are never handed to us: `CGEventPost` autoreleases them internally, so no reference exists to hold and no `CFRelease` was skipped. `CGEvent` and `CGEventSource` are both released correctly by their `foreign_type!` `Drop`, and that is not what this is about.

Nothing drains a plain thread. The main thread is unaffected because `CFRunLoop` pushes and pops a pool around each iteration, and so is the tap thread, which is inside a run loop of its own. The worker thread runs `block_on` and has no pool at any point in its life.

## Shape after

`Emitter::post` runs its body inside an autorelease pool, so the two objects the post autoreleases are freed when it returns.

```rust
impl Emitter {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// The event states its own modifiers rather than trusting a source: whoever built it said
    /// what it carries, and we apply exactly that. See [`keyboard_event`].
    ///
    /// The body runs inside an autorelease pool because `CGEventPost` autoreleases two
    /// `CFData`s per call, about 574 bytes. An `Emitter` posts from whatever thread owns it,
    /// which for a daemon is a worker thread with no pool of its own, so a pool here is what
    /// makes a post free what it allocated. Draining per post rather than per batch keeps the
    /// property local to the call that needs it: pushing and popping a pool is tens of
    /// nanoseconds against a post that costs tens of microseconds.
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        autoreleasepool(|_pool| {
            let event = keyboard_event(&self.source, key, press, flags)?;
            self.tag.stamp(&event);
            event.post(CGEventTapLocation::Session);
            Ok(())
        })
    }
}
```

`objc2::rc::autoreleasepool` is a safe function, so the crate keeps `unsafe_code = "forbid"` from the workspace and gains no `#[expect(unsafe_code)]`. Its signature is

```rust
pub fn autoreleasepool<T, F>(f: F) -> T
where
    for<'pool> F: AutoreleaseSafe + FnOnce(AutoreleasePool<'pool>) -> T,
```

so the closure returns the `Result<(), EmitError>` that `post` returns, and the pool is dropped before that value comes back. The pool handle itself is unused: nothing in the body needs to name a lifetime tied to it.

## Change 1: a pool around the post

Files: `crates/freddie_keyboard/Cargo.toml`, `crates/freddie_keyboard/src/sys/macos.rs`.

### Cargo.toml before

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = { version = "0.25", features = ["link"] }
core-foundation = { version = "0.10", features = ["link"] }
freddie_hid_device = { path = "../freddie_hid_device", version = "0.0.1" }
```

### Cargo.toml after

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = { version = "0.25", features = ["link"] }
core-foundation = { version = "0.10", features = ["link"] }
# For `rc::autoreleasepool`. `CGEventPost` autoreleases, and an `Emitter` posts from whatever
# thread owns it, which is not required to have a pool. The one function used is safe, so this
# does not cost the crate its `unsafe_code = "forbid"`.
objc2 = "0.6"
freddie_hid_device = { path = "../freddie_hid_device", version = "0.0.1" }
```

`objc2` 0.6 is already in the dependency tree through `freddie_app_nav`, `freddie_windows`, `freddie_menu_bar`, `freddie_overlay` and `freddie_main_loop`, so this adds no new crate to the lock file.

### The import

```rust
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use freddie_hid_device::{DeviceInfo, ResolveFailure, SourceId, resolve, source_of};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};
use objc2::rc::autoreleasepool;
```

### `Emitter::post` before

```rust
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        let event = keyboard_event(&self.source, key, press, flags)?;
        self.tag.stamp(&event);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
```

### `Emitter::post` after

The body in "Shape after", with the doc comment there.

`emit` and `tap` are unchanged: both already route through `post`, so a chord gets one pool per half, which is one pool per posted event either way.

## Call sites

None change. `Emitter::emit` and `Emitter::tap` keep their signatures, and `Emitter` stays `!Send` for the reason it already is. Mercury and figaro need no edit.

## Verification

`cargo test -p freddie_keyboard` still passes; no test asserts this. The leak exists only after a real `CGEventPost`, so reaching it from a test means injecting a key into whatever is focused on every test run and injecting nothing in CI, where the runner has no Accessibility grant. It is checked against the running daemon instead, the same way the source mapping was.

After `mercury restart`:

1. Note the allocation count and footprint:

```
PID=$(pgrep -f 'mercury daemon')
vmmap -summary $PID | grep -E 'Physical footprint:|DefaultMallocZone'
```

2. Type a few hundred keys, or drive them in, and count the `post` records for that pid in `~/Library/Logs/mercury/mercury.log`.
3. Read the two numbers again. Before this change the allocation count rises by two per post and the footprint by about 570 bytes per post. After it, both hold flat: a thousand posts move the allocation count by single digits and the footprint not at all.

`heap $PID` is the confirmation if the numbers disagree: `CFData` and `CFData (Bytes Storage)` are the two counts that used to climb together, one per post each.

## Ordered commits

1. Change 1: `objc2` on freddie_keyboard, `autoreleasepool` around `Emitter::post`.
