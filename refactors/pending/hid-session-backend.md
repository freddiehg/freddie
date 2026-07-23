# freddie_keyboard_hid: the interface crate

The new crate figaro depends on, exposing the same `intercept`/`Interceptor`/`Emitter` as `freddie_keyboard`, but backed by `freddie_hidd`'s socket instead of a CGEventTap: the interceptor's reader turns each `Uplink::Input` into an `on_key` call, and the emitter turns `emit`/`tap` into `Downlink::Emit`s. This is the first, minimal change. It compiles and figaro can depend on it with the emitter logic unit-tested; it does nothing end to end until `freddie_hidd` exists (`hidd.md`).

Nothing above `intercept` changes. figaro is the mercury shape against this crate; its model, event loop, effect loop, `freddie_app_nav`, and menu bar are untouched.

## The crate

```toml
# crates/freddie_keyboard_hid/Cargo.toml
[package]
name = "freddie_keyboard_hid"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }
freddie_hid_wire = { path = "../freddie_hid_wire", version = "0.0.1" }
tracing = "0.1"

[lints]
workspace = true
```

Under `forbid(unsafe_code)`: it is a socket client. It re-exports the vocabulary and defines the same error types as `freddie_keyboard`, so a consumer's `use` and call sites match either crate:

```rust
// crates/freddie_keyboard_hid/src/lib.rs
pub use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

/// The keyboard could not be grabbed. On HID this means `freddie_hidd` is unreachable.
pub struct CaptureError;   // same shape as freddie_keyboard::CaptureError

/// A key could not be emitted.
pub enum EmitError { Unmappable(Key), Post }
```

The two error types are duplicated across the two crates rather than shared, keeping this change to one new crate that touches nothing else.

## freddie_hid_wire

The session↔daemon protocol, a small crate this one and `freddie_hidd` both depend on. Two directions, each an enum so a new message is a variant, not a format break:

```rust
// crates/freddie_hid_wire/src/lib.rs
use freddie_keys::{Key, KeyEvent, PressType};
use serde::{Deserialize, Serialize};

/// Daemon → session. Each physical key, as the model's `on_key` sees it: a full `KeyEvent`
/// carrying the input-side modifier flags the daemon tracks, so the session sees exactly what
/// the CGEventTap backend delivers.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Uplink {
    Input(KeyEvent),
}

/// Session → daemon. One output key transition. No flags: the emitter here has already
/// expanded modifier state into explicit modifier-key transitions, so the daemon only toggles
/// a held key and never interprets flags on this path.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Downlink {
    Emit { key: Key, press: PressType },
}

/// Read/write one framed message: a `u32` little-endian length prefix and a JSON body.
/// Keystroke rates make JSON's cost irrelevant, and it matches the repo's JSON-record logging.
pub fn write_msg<T: Serialize>(sock: &mut impl std::io::Write, msg: &T) -> std::io::Result<()>;
pub fn read_msg<T: DeserializeOwned>(sock: &mut impl std::io::Read) -> std::io::Result<T>;

/// A frame past this is a protocol error; nothing legitimate on this socket is large.
const MAX_FRAME_BYTES: usize = 64 * 1024;
```

Prefactor within this change: `freddie_keys::{Key, PressType, ModifierFlags, KeyEvent}` derive `Serialize`/`Deserialize`. They are plain data; the derives are additive and serve both the wire and, later, structured logging.

## intercept

```rust
/// Connect to `freddie_hidd` and hand back the interceptor and emitter. `on_key` runs on the
/// reader thread for each physical key the daemon forwards.
///
/// # Errors
///
/// [`CaptureError`] if the daemon socket cannot be reached: `freddie_hidd` is not running,
/// or this user is not the one it was installed for.
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    let sock = UnixStream::connect(daemon_socket_path()).map_err(|_| CaptureError)?;
    let writer = sock.try_clone().map_err(|_| CaptureError)?;

    let reader = std::thread::spawn(move || {
        let mut sock = sock;
        loop {
            match freddie_hid_wire::read_msg::<Uplink>(&mut sock) {
                Ok(Uplink::Input(event)) => {
                    // The return is ignored: there is no chain to return a key into. A key the
                    // model passes is one it re-emits through the Emitter, never one it returns.
                    let _ = on_key(event);
                }
                Err(_) => break, // daemon closed the socket; the interceptor is done
            }
        }
    });

    let interceptor = Interceptor { _reader: ReaderThread::new(reader) };
    let emitter = Emitter { writer: Arc::new(Mutex::new(EmitState::new(writer))) };
    Ok((interceptor, emitter))
}
```

`on_key`'s `Option<KeyEvent>` return is part of the shared signature and is ignored here, exactly as the CGEventTap backend's return path is unused by mercury today. Both backends are observe-plus-emit.

## Interceptor

```rust
/// An active grab over the HID daemon. Dropping it closes the socket, which ends the reader
/// thread and tells the daemon to revert to passthrough.
pub struct Interceptor {
    _reader: ReaderThread,
}
```

`ReaderThread` shuts the socket down on drop (`UnixStream::shutdown`) so the blocking `read_msg` returns and the thread joins under a bounded timeout, the shape `TapThread` uses in `sys/macos.rs`. The daemon sees the disconnect and resumes passing keys through, so dropping the interceptor leaves a live keyboard.

## Emitter

The emitter holds the one piece of state the HID path needs that the CGEventTap path does not: the modifier keys it has currently pressed on the wire, because the virtual device is driven by explicit modifier-key transitions, not by flags stamped on each event. The daemon stays dumb; this is where flags become modifier holds.

```rust
pub struct Emitter {
    writer: Arc<Mutex<EmitState>>,
}

struct EmitState {
    sock: UnixStream,
    /// The modifier keys currently emitted as held. `emit`/`tap` reconcile the wire to the
    /// flags an event states by pressing or releasing modifier keys through this.
    held_mods: ModifierFlags,
}
```

`emit`, the single transition used for passthrough:

```rust
/// Emit one key transition carrying `flags`. Same signature as the CGEventTap emitter.
pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
    let mut st = self.writer.lock().expect("emitter");
    if is_modifier(key) {
        // A modifier delivered as its own key (passthrough of the real stream). Forward it and
        // track it; its flags are itself.
        st.send(key, press)?;
        st.held_mods.set(flag_of(key), press == PressType::Down);
    } else {
        st.reconcile_to(flags)?;   // press/release modifier keys so the wire matches `flags`
        st.send(key, press)?;
    }
    Ok(())
}
```

`tap`, the self-contained chord:

```rust
/// Press then release `key` under `flags`, then restore the modifiers to what they were.
/// A synthesized chord (cmd-r) leaves no modifier held afterward, unlike a passthrough key.
pub fn tap(&self, key: Key, flags: ModifierFlags) -> Result<(), EmitError> {
    let mut st = self.writer.lock().expect("emitter");
    let before = st.held_mods;
    st.reconcile_to(before | flags)?;  // add the chord's modifiers on top of any held
    st.send(key, PressType::Down)?;
    st.send(key, PressType::Up)?;
    st.reconcile_to(before)?;          // release only what the chord added
    Ok(())
}
```

`reconcile_to` and `send`:

```rust
impl EmitState {
    /// Press or release modifier keys until `held_mods == target`, sending each transition.
    fn reconcile_to(&mut self, target: ModifierFlags) -> Result<(), EmitError> {
        for flag in MODIFIER_FLAGS {                 // CONTROL, COMMAND, ALT, SHIFT
            let have = self.held_mods.contains(flag);
            let want = target.contains(flag);
            if have != want {
                let press = if want { PressType::Down } else { PressType::Up };
                self.send(modifier_key(flag), press)?;   // the left-side key for the flag
                self.held_mods.set(flag, want);
            }
        }
        Ok(())
    }

    fn send(&mut self, key: Key, press: PressType) -> Result<(), EmitError> {
        freddie_hid_wire::write_msg(&mut self.sock, &Downlink::Emit { key, press })
            .map_err(|_| EmitError::Post)
    }
}
```

`reconcile_to` presses the left-side modifier key for a flag (`COMMAND` → `MetaLeft`, and so on), since `ModifierFlags` is unsided. A modifier the model emits explicitly by side (`MetaRight`) goes through `emit`'s modifier branch and is forwarded with its side intact.

Two limits stated so they are not surprises:

- `ModifierFlags::FN` is not reconcilable: there is no `fn` in the Keyboard/Keypad usage page (Apple carries it on a vendor top-case page). A `tap` whose flags include `FN` cannot press `fn` on HID and emits without it. Emitting `fn` is a later change against `freddie_virtual_hid`'s other `post_*` reports.
- A `tap(key, flags)` fired while a conflicting modifier is physically held emits the union (`before | flags`), so the chord runs with the held modifier still down rather than spuriously releasing a key the user is holding. A chord that must suppress a held modifier is not expressible this way; no current binding needs it.

## Errors

`CaptureError` and `EmitError` are this crate's own, defined above to match `freddie_keyboard`'s. `CaptureError` from `intercept` means the daemon is unreachable; `EmitError::Post` from a send means the socket died mid-run (the daemon exited), which the effect loop logs exactly as it logs a CGEventTap post failure.

## Tests

The pure logic is the reconciliation, and it is testable with a fake sink instead of a socket:

- `emit` of a non-modifier under `flags` sends modifier-key downs to reach `flags`, then the key; a following `emit` under the same flags sends no modifier events.
- `tap(R, COMMAND)` from no held modifiers sends `MetaLeft` down, `R` down, `R` up, `MetaLeft` up, in order.
- `tap(R, COMMAND)` with `SHIFT` already held leaves `SHIFT` held afterward and adds/removes only `COMMAND`.
- passthrough: `emit(MetaLeft, Down, ..)` forwards `MetaLeft` down and sets `held_mods`; a later non-modifier key under `COMMAND` sends no extra `MetaLeft`.
- `emit(Key::Raw(u16), ..)` forwards the raw key unchanged for the daemon to post as that usage.

The live path (figaro remapping a key in a password field, which CGEventTap cannot reach) is the manual demo that proves the backend, run against a `freddie_hidd` installed per `hidd.md`.
