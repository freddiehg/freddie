# freddie_keyboard on HID: the session backend

`freddie_keyboard`'s second macOS backend, `sys/hid.rs`, selected by the `hid` Cargo feature. It implements the same `intercept`/`Interceptor`/`Emitter` as `sys/macos.rs`, but instead of a CGEventTap it connects to `freddie_hidd`'s socket: the interceptor's reader turns each `Uplink::Input` into an `on_key` call, and the emitter turns `emit`/`tap` into `Downlink::Emit`s. Depends on `freddie_hid_wire` (defined in `hidd.md`).

Nothing above `intercept` changes. figaro is the mercury shape with this backend selected; its model, event loop, effect loop, `freddie_app_nav`, and menu bar are untouched.

## The feature

```toml
# crates/freddie_keyboard/Cargo.toml
[features]
default = ["cgevent"]
cgevent = []
hid = ["dep:freddie_hid_wire"]

[dependencies]
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }
freddie_hid_wire = { path = "../freddie_hid_wire", version = "0.0.1", optional = true }
tracing = "0.1"

[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = { version = "0.25", features = ["link"] }
core-foundation = { version = "0.10", features = ["link"] }
```

`core-graphics`/`core-foundation` stay unconditional today; a later change can gate them behind `cgevent` so an `hid`-only build does not pull them. Not required for correctness, so it is not part of this change.

Backend selection, `sys/mod.rs`:

```rust
// before
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Emitter, Interceptor, intercept};

#[cfg(not(target_os = "macos"))]
compile_error!("freddie_keyboard only has a macOS backend so far");
```

```rust
// after
#[cfg(all(target_os = "macos", feature = "cgevent"))]
mod macos;
#[cfg(all(target_os = "macos", feature = "cgevent"))]
pub use macos::{Emitter, Interceptor, intercept};

#[cfg(all(target_os = "macos", feature = "hid"))]
mod hid;
#[cfg(all(target_os = "macos", feature = "hid"))]
pub use hid::{Emitter, Interceptor, intercept};

#[cfg(all(target_os = "macos", not(any(feature = "cgevent", feature = "hid"))))]
compile_error!("freddie_keyboard on macOS needs exactly one of `cgevent` or `hid`");
#[cfg(all(target_os = "macos", all(feature = "cgevent", feature = "hid")))]
compile_error!("freddie_keyboard: `cgevent` and `hid` are mutually exclusive");

#[cfg(not(target_os = "macos"))]
compile_error!("freddie_keyboard only has a macOS backend so far");
```

The two features are mutually exclusive because they export the same names. mercury keeps the default; figaro sets `default-features = false, features = ["hid"]`.

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

`CaptureError` and `EmitError` are the crate's existing types, unchanged. `CaptureError` from `intercept` means the daemon is unreachable; `EmitError::Post` from a send means the socket died mid-run (the daemon exited), which the effect loop logs exactly as it logs a CGEventTap post failure.

## Tests

The pure logic is the reconciliation, and it is testable with a fake sink instead of a socket:

- `emit` of a non-modifier under `flags` sends modifier-key downs to reach `flags`, then the key; a following `emit` under the same flags sends no modifier events.
- `tap(R, COMMAND)` from no held modifiers sends `MetaLeft` down, `R` down, `R` up, `MetaLeft` up, in order.
- `tap(R, COMMAND)` with `SHIFT` already held leaves `SHIFT` held afterward and adds/removes only `COMMAND`.
- passthrough: `emit(MetaLeft, Down, ..)` forwards `MetaLeft` down and sets `held_mods`; a later non-modifier key under `COMMAND` sends no extra `MetaLeft`.
- `emit(Key::Raw(u16), ..)` forwards the raw key unchanged for the daemon to post as that usage.

The live path (figaro remapping a key in a password field, which CGEventTap cannot reach) is the manual demo that proves the backend, run against a `freddie_hidd` installed per `hidd.md`.
