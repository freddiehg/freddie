# the HID keyboard backend

A new crate, `freddie_keyboard_hid`, exposes the same `intercept`/`Interceptor`/`Emitter` interface as `freddie_keyboard`, but backed by a physical-keyboard seize at the IOKit HID level and emission through Karabiner's virtual HID device. mercury keeps depending on `freddie_keyboard` (CGEventTap); figaro depends on `freddie_keyboard_hid`. `freddie_keyboard` is not touched and not removed.

Two crates side by side, rather than one crate with a Cargo feature: the interface is the shared `freddie_keys` vocabulary plus matching `intercept`/`Interceptor`/`Emitter` signatures, and a consumer picks a backend by which crate it depends on. figaro is its own workspace, so it just names `freddie_keyboard_hid`; nothing about mercury's build changes, and there is no feature to unify.

This doc is the map. The concrete work lives in four others:

- `hid-session-backend.md` — `freddie_keyboard_hid`, the new crate with the interface: `intercept`/`Interceptor`/`Emitter` implemented over the daemon socket. No unsafe. It defines `freddie_hid_wire`, the session↔daemon protocol. This is the first, minimal change.
- `hid-virtual-device-client.md` — `freddie_virtual_hid`, the pure-Rust client to Karabiner's virtual-HID daemon (output). No unsafe, no C++.
- `hid-seize.md` — `freddie_hid_sys`, the leaf crate that seizes and reads the physical keyboard (input). The only place unsafe lives.
- `hidd.md` — `freddie_hidd`, the root LaunchDaemon that wires seize to virtual device and serves the session socket. Karabiner's own `Karabiner-VirtualHIDDevice-Daemon` is not auto-launched once Karabiner-Elements is gone, so the install carries a LaunchDaemon plist that keeps it running.

## Why two processes

Seizing a keyboard (`IOHIDManagerOpen` with `kIOHIDOptionsTypeSeizeDevice`) requires root, and root does not exempt the process from the Input Monitoring TCC grant either (Apple TN2187; Karabiner's `DEVELOPMENT.md`). The figaro model, `freddie_app_nav`, and the menu bar need an Aqua session (window server, `NSWorkspace`, per-user TCC), which a root daemon at the login window does not have — the same split `crates/mercury/src/agent.rs` already documents for the CGEventTap agent.

So the seize lives in a root `LaunchDaemon` (system domain) and everything else stays in the session `LaunchAgent` (Aqua), exactly where mercury has it. The daemon is a thin transport: it does not run the model. It reads the physical keyboard, forwards each key up to the session, receives the session's output keys back, and posts them to the virtual device.

```
hardware key
  → freddie_hidd: seize (freddie_hid_sys)         [root LaunchDaemon]
  → freddie_hid_wire uplink (unix socket)
  → figaro: on_key runs the model                 [session LaunchAgent, Aqua]
  → figaro: Emitter sends output
  → freddie_hid_wire downlink (unix socket)
  → freddie_hidd: post report (freddie_virtual_hid)
  → Karabiner-VirtualHIDDevice-Daemon             [root, pqrs-signed]
  → virtual HID keyboard → the system
```

The session sees the identical `intercept`/`Interceptor`/`Emitter` it sees on CGEventTap. The whole difference is below `intercept`.

## Backend selection

By dependency. mercury stays on `freddie_keyboard`; figaro names the new crate:

```toml
# figaro/Cargo.toml
freddie_keyboard_hid = { path = "../freddie/crates/freddie_keyboard_hid" }
```

Both crates export `intercept`, `Interceptor`, `Emitter`, `CaptureError`, and `EmitError`, and both re-export `Key`/`KeyEvent`/`PressType`/`ModifierFlags` from `freddie_keys`, so a consumer's `use` line and call sites are identical whichever it depends on. The parity is by convention over the shared vocabulary, not enforced by a trait; a `freddie_keyboard_api` trait crate could enforce it later, but it is machinery this does not need yet and is out of scope for the first change.

## The interface is already right for this

`intercept(on_key)` is observe-plus-emit. The HID backend ignores `on_key`'s `Option<KeyEvent>` return: there is no tap chain to return a key into, so a passed key is one the session re-emits through the `Emitter`, never one it returns. mercury's CGEventTap usage already returns `None` for every key and drives all output through the `Emitter`, so both consumers are observe-plus-emit and the two backends present the same surface. `freddie_keys::Key`, `KeyEvent`, `PressType`, and `ModifierFlags` cross the wire unchanged.

`Key::Raw(u16)` stays the one non-portable value (per `freddie-keyboard-cross-platform.md`): on CGEventTap it is a `CGKeyCode`, on HID it is a Keyboard/Keypad usage id. A key named on one backend can be `Raw` on the other; that is the existing accepted cost of an abstract `Key`.

## Decisions

These are settled; the component docs assume them.

- The HID backend is its own crate, `freddie_keyboard_hid`, not a feature of `freddie_keyboard`. A consumer selects a backend by dependency.
- Reuse Karabiner's installed `Karabiner-DriverKit-VirtualHIDDevice` for output. No DriverKit system extension of our own, so no Apple-granted entitlement and no notarization. figaro is a personal tool; distribution to other machines is out of scope.
- The Karabiner client is pure Rust over the daemon's unix socket, not the `karabiner-driverkit` crate (which compiles a C++23 shim through `cc`). The protocol is plain bytes; keeping it in Rust keeps `freddie_virtual_hid` under `forbid(unsafe_code)`.
- Physical-key input is read as HID input values (usage page + usage + down/up), not raw input reports. Values normalize across every keyboard's report format; raw reports would need each device's report descriptor. `freddie_hid_sys` exposes values.
- The session↔daemon socket is restricted to the target user's uid (the socket carries every keystroke in both directions, so it is a keylogger and a key-injector to anyone who can open it). `hidd.md` specifies the filesystem permissions and the peer-uid check.
- The model runs in the session, never in the root daemon. This is what makes the session-side API identical to CGEventTap.

## New crates

- `freddie_keyboard_hid` — safe Rust, the interface crate: `intercept`/`Interceptor`/`Emitter` over the daemon socket. What figaro depends on.
- `freddie_hid_wire` — safe Rust, the session↔daemon frame types over `freddie_keys`. Shared by `freddie_keyboard_hid` and `freddie_hidd`.
- `freddie_virtual_hid` — safe Rust, the Karabiner daemon client.
- `freddie_hid_sys` — leaf, opts out of `forbid(unsafe_code)`, wraps `io-kit-sys`. Seize and read.
- `freddie_hidd` — the root daemon binary.

## Order

The first change is minimal and interface-first: stand up `freddie_keyboard_hid` with the interface. The heavy pieces follow and make it live.

1. `freddie_keyboard_hid` and `freddie_hid_wire` (`hid-session-backend.md`), plus the `freddie_keys` serde prefactor. The new crate with the same interface: `intercept` connects to the daemon socket, `Emitter` sends over it. No unsafe. It compiles and figaro can depend on it, with the emitter's modifier-reconciliation unit-tested; it does nothing end to end until the daemon lands in step 4. This is the small, self-contained first step.
2. `freddie_virtual_hid` (`hid-virtual-device-client.md`). Demo: a test binary types a string through the virtual device. Needs the Karabiner driver installed and its daemon running; needs no seize.
3. `freddie_hid_sys` (`hid-seize.md`). Demo: a root test binary seizes the keyboard and logs every key, and the keys stop reaching the system while it runs.
4. `freddie_hidd` (`hidd.md`), combining 2 and 3, plus the session socket, the LaunchDaemon plists, and the install verb. Demo: with no session client attached, the daemon echoes (posts what it reads) to prove the input-to-output loop end to end; then `freddie_keyboard_hid` from step 1 drives it and a remap shows up in a password field, which CGEventTap cannot do.

## Scope

figaro replaces Karabiner-Elements: on HID it holds the exclusive seize, so Karabiner-Elements' grabber is not running alongside it. Migrating what `voicemode`'s `karabiner.edn` does today into figaro's model is separate work and not part of this change. What this change does carry, because nothing else will once Karabiner-Elements is not running, is launching Karabiner's `VirtualHIDDevice-Daemon` (the output side), in `hidd.md`.

## Known platform risk

The IOKit seize path is what Apple is tightening. macOS 26.4 reportedly broke `kIOHIDOptionsTypeSeizeDevice` on the built-in MacBook keyboard specifically (Apple Developer Forums 817003); external keyboards and the general path still work, and Karabiner already had to move its output to DriverKit for adjacent reasons. Input seizing remains IOHIDManager and remains the only non-DriverKit option, but this backend is betting against a platform that is closing the door slowly.
