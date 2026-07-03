# capturing keyboard events

Three primitives: capture, emit, drop. A `Capture` intercepts keys while it is alive; `emit` posts a key or chord; dropping the `Capture` releases its keys. Multiple captures stack and work together.

## The API

```rust
// freddie_keyboard

pub use freddie_keys::{Keyboard, KeyEvent};

pub struct Capture(/* private */);
impl Capture {
    pub fn grab() -> Result<Capture, CaptureError>;   // capture every key
    pub fn builder() -> Builder;                       // capture only chosen keys
    pub fn recv(&self) -> Option<KeyEvent>;            // blocks; None once released
}
impl Iterator for Capture { type Item = KeyEvent; }   // for key in capture
// impl Drop for Capture: releases its keys.

pub struct Builder(/* private */);
impl Builder {
    pub fn key(self, key: Keyboard) -> Builder;
    pub fn grab(self) -> Result<Capture, CaptureError>;
}

pub fn tap(key: Keyboard) -> Result<(), EmitError>;         // press then release; returns nothing
pub fn press(key: Keyboard) -> Result<Press, EmitError>;    // held until the last Press for this key drops
pub struct Press(/* ref-counted hold on the key */);
// impl Drop for Press: the last one out releases the key.

pub struct CaptureError;                       // Display + Error
pub enum EmitError { Source, Post, Unmappable(Keyboard) }
```

This is a general keyboard library; mercury is one consumer. The library carries the whole surface (capture-all, the selective `builder`, and held presses) so others can use it; mercury exercises only `Capture::grab()` and `tap`.

Everything is individual key events, and there are no chords as a real thing. `tap` presses and releases a key and returns nothing. `press` presses a key and returns a `Press`; the key is released when that `Press` drops. Presses of the same key are ref-counted, so it stays down until the last `Press` drops, and a crash or early return releases everything held. That guard is the only concession to chords: cmd+r is `let _cmd = press(cmd)?; tap(r)?;`, with `_cmd` releasing cmd at the end of the scope. No `Chord` type, no modifier list.

`recv` is a blocking sync call, and the internal channel should be a runtime-agnostic async one (`flume`, `async-channel`), not `std::sync::mpsc`. Its `send` stays sync for the tap thread, `recv` blocks for a plain consumer, and `recv_async` awaits in a tokio task. That is one channel hop, the same as wiring tokio in from the start, but without baking tokio into the library. Draining a `std::sync::mpsc` into a tokio channel on a bridge thread is a second hop and an extra thread for nothing. At keyboard rates none of this is measurable, microseconds against tens of milliseconds between keys, so the single hop is worth it for fewer moving parts, not speed.

```rust
let all  = Capture::grab()?;                                  // every key
let some = Capture::builder().key(KeyR).key(KeyN).grab()?;    // R and N; the rest pass through
```

## Stacking

Several captures coexist. Each key is routed to at most one of them; a key no capture asked for passes through to the app.

- `Capture::grab()` takes every key.
- `Capture::builder()...grab()` takes only its listed keys and lets the rest through.
- The set intercepted is the union of every live capture's keys.
- A key more than one capture asked for goes to the newest.
- Dropping a capture removes its keys from the union, so they pass through again (or fall to an older capture that also asked for them).

That is what "they stack and transparently work together" means: two selective captures over different keys each get their own, and everything else is untouched. A capture-all sits under them and takes whatever they did not claim.

One shared tap, one registry, and a pure router carry this:

```rust
// captures registered newest-first; grab() registers a wildcard.
fn route(reg: &Registry, key: Keyboard) -> Option<CaptureId> {
    reg.newest_owner(key)   // Some(id): that capture swallows + receives it. None: pass through.
}
```

The tap installs on the first `grab` and tears down when the last capture drops. The registry adds a capture's keys on `grab` and removes them on drop.

The builder only lists `Keyboard` values, so a selective capture's set is exactly those keys. Whether to also cap `grab`'s wildcard to keys the table knows is open, and not needed for v1.

## Unit tests

Routing is a pure function over the registry, so the multi-capture behavior is unit-tested with no tap:

- Two captures over disjoint keys: each routes to its own; a third key routes to `None` (passthrough).
- A capture-all plus a selective capture: the selective one owns its keys (newest), the capture-all owns the rest.
- Overlapping keys route to the most-recently-grabbed capture.
- After dropping a capture, its keys route to `None`, or to an older capture that also registered them.

## Loop prevention

`emit` stamps each event's `USER_DATA` field with a module constant; `route` short-circuits a tagged event to `Keep` and never routes it. Without that, `emit` posts a key, the tap sees it, routes and re-dispatches it, which emits again. It is loop-prevention, not passthrough.

## The macOS backend

```rust
// grab(): register the capture, install the shared tap once, hand back the receiver.
CGEventTap::with_enabled(
    Session, HeadInsertEventTap, Default, [KeyDown, KeyUp],
    |_, kind, event| {
        if event.field(USER_DATA) == TAG { return Keep; }        // our own emit
        let key = from_code(event.field(KEYCODE));
        match key.and_then(|k| route(&registry, k)) {
            Some(id) => { registry.send(id, KeyEvent { key, down }); Drop }
            None     => Keep,                                     // no capture wants it
        }
    },
    || { ready.send(CFRunLoop::get_current()); CFRunLoop::run_current(); },
);
```

`with_enabled` does the run-loop wiring inside core-graphics, so we call no `unsafe`. `CFRunLoop` is `Send`, so dropping the last capture stops the loop from any thread. A process exit or `kill -9` also releases the tap, since the WindowServer drops taps for dead processes.

## Emitting

Held keys are ref-counted per key: `press` holds, and the last `Press` to drop releases. Default is `Rc` with a thread-local map, so `Press` is `!Send` and emitting stays on one thread. An `arc` Cargo feature swaps `Rc` for `Arc` and the thread-local `RefCell` for a `static Mutex`, making `Press: Send` for consumers that emit from several threads.

```rust
thread_local! { static HELD: RefCell<HashMap<Keyboard, Weak<Hold>>>; }   // Mutex under `arc`

pub fn press(key: Keyboard) -> Result<Press, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    HELD.with_borrow_mut(|held| {
        if let Some(hold) = held.get(&key).and_then(Weak::upgrade) {
            return Ok(Press(hold));             // already down: share the hold
        }
        post(code, /* down */ true)?;           // 0 -> 1: key-down, stamped USER_DATA = TAG
        let hold = Rc::new(Hold { key });       // Arc under `arc`
        held.insert(key, Rc::downgrade(&hold));
        Ok(Press(hold))
    })
}

pub struct Press(Rc<Hold>);                     // !Send by default; Send under `arc`
struct Hold { key: Keyboard }
impl Drop for Hold {                            // last owner emits the release
    fn drop(&mut self) {
        if let Some(code) = to_code(self.key) {
            let _ = post(code, false);          // Drop cannot return an error
        }
    }
}

pub fn tap(key: Keyboard) -> Result<(), EmitError> {
    press(key)?;                                // the Press drops here: down then up
    Ok(())
}
```

README TODO: consider making `arc` the default. mercury emits from one thread, so `Rc` is fine for now.

## Left, right, and modifiers

`Keyboard` splits the sides already: `ShiftLeft`/`ShiftRight`, `ControlLeft`/`ControlRight`, `AltLeft`/`AltRight`, `MetaLeft`/`MetaRight`, each with its own macOS keycode, so capture reports the exact key. But modifiers do not arrive as `KeyDown`/`KeyUp`; macOS sends `FlagsChanged`. So the tap listens for `[KeyDown, KeyUp, FlagsChanged]`, and for a `FlagsChanged` it reads the keycode (which carries the side) and infers press or release from whether that modifier's flag bit just turned on or off.

## Custom and unmapped keys

macOS keycodes are arbitrary `u16`: `CGEventCreateKeyboardEvent` takes any code and the tap reads any code, so made-up keys can be captured and emitted. A code produces a character only if the active layout maps it, so a custom code is a remap intermediary (emit it, catch it in your own tap), not typed text. `Keyboard::Raw(u16)` carries such a code, and `from_code` returns `Raw(code)` for anything the named table lacks, so no key is dropped for being unnamed. The `u16` is the native code, which makes `Raw` the one part that is not portable across OSes. Linux and Windows are scancode/keycode based too, so likely the same, but that is unconfirmed.

## Escape and quitting are not here

`freddie_keyboard` has no notion of escape or quit. That is the consumer's, jerry-rigged until the hijack is trusted:

```rust
for ev in capture {
    if ev.key == Keyboard::Escape { process::exit(0); }   // mercury's, not the keyboard's
    if ev.down { event_tx.send(key(ev.key)); }
}
```

## Ordering caveat

A selective capture passes non-captured keys natively (`Keep`) while its captured keys arrive through `recv` asynchronously. If the consumer re-emits captured keys, they can land out of order against the passed-through ones (passthrough.md). Fine when captured keys are consumed as commands; watch it when remapping inline. mercury uses capture-all, so nothing passes through and there is no reorder.

## What's left to build

The code today has `run(on_key)` + `emit(key)` + `emit_chord(mods, key)` on the core-graphics backend, with mercury driving it. To reach the API above:

1. `Capture` (`grab` + `builder`) over one shared tap, a registry, and `route`; `recv` / `Iterator` / `Drop`.
2. Replace `emit(key)` + `emit_chord` with `tap(key)` and `press(key) -> Press` (ref-counted holds, `Rc` under the default, `Arc` under `arc`).
3. mercury: reader loop `for ev in capture { ... }` over a capture-all; effect loop `tap(k)`, and cmd+r as `let _cmd = press(Keyboard::MetaLeft)?; tap(k)?;`.

## Known v1 limits

- A key with no named `Keyboard` mapping is delivered as `Raw(code)`, so nothing is lost, but that code is the non-portable macOS value.
- If macOS disables the tap on a timeout it does not auto re-enable (the cost of `with_enabled`, which keeps us unsafe-free). The callback is trivial, so it should not fire.
- F21-F24 have no macOS keycode, so `emit` returns `Unmappable` for them until we learn what real hardware sends.

## Permissions

An active tap needs Accessibility plus Input Monitoring, granted to whatever launches the binary. First run prompts; flip it in System Settings and relaunch.
