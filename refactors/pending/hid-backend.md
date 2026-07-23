# the HID keyboard backend

`freddie_keyboard` gains a second macOS backend that seizes the physical keyboard at the IOKit HID level and emits through Karabiner's virtual HID device, behind the same `intercept`/`Interceptor`/`Emitter` API the CGEventTap backend already exposes. mercury stays on CGEventTap; figaro selects HID. The CGEventTap backend is not touched and not removed.

This doc is the map. The concrete work lives in five others, each independently shippable:

- `hid-virtual-device-client.md` — `freddie_virtual_hid`, the pure-Rust client to Karabiner's virtual-HID daemon (output). No unsafe, no C++.
- `hid-seize.md` — `freddie_hid_sys`, the leaf crate that seizes and reads the physical keyboard (input). The only place unsafe lives.
- `hidd.md` — `freddie_hidd`, the root LaunchDaemon that wires seize to virtual device and exposes a socket to the session. Defines `freddie_hid_wire`, the session↔daemon protocol.
- `hid-session-backend.md` — `freddie_keyboard`'s `sys/hid.rs` and the `hid` Cargo feature, implementing `intercept`/`Interceptor`/`Emitter` over that socket.
- Karabiner's own `Karabiner-VirtualHIDDevice-Daemon` is not auto-launched once Karabiner-Elements is gone; `hidd.md` carries the LaunchDaemon plist that keeps it running.

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

A Cargo feature on `freddie_keyboard`, resolved per consumer:

```toml
# freddie_keyboard/Cargo.toml
[features]
default = ["cgevent"]
cgevent = []
hid = ["dep:freddie_hid_wire"]
```

mercury lives in this workspace and keeps the default (`cgevent`). figaro is a separate workspace with its own lockfile, so it selects `hid` without forcing it on mercury:

```toml
# figaro/Cargo.toml
freddie_keyboard = { path = "../freddie/crates/freddie_keyboard", default-features = false, features = ["hid"] }
```

`sys/mod.rs` selects on the feature within macOS; details in `hid-session-backend.md`. Exactly one of `cgevent`/`hid` must be active on macOS, enforced by `compile_error!`.

## The interface is already right for this

`intercept(on_key)` is observe-plus-emit. The HID backend ignores `on_key`'s `Option<KeyEvent>` return: there is no tap chain to return a key into, so a passed key is one the session re-emits through the `Emitter`, never one it returns. mercury's CGEventTap usage already returns `None` for every key and drives all output through the `Emitter`, so both consumers are observe-plus-emit and the two backends present the same surface. `freddie_keys::Key`, `KeyEvent`, `PressType`, and `ModifierFlags` cross the wire unchanged.

`Key::Raw(u16)` stays the one non-portable value (per `freddie-keyboard-cross-platform.md`): on CGEventTap it is a `CGKeyCode`, on HID it is a Keyboard/Keypad usage id. A key named on one backend can be `Raw` on the other; that is the existing accepted cost of an abstract `Key`.

## Decisions

These are settled; the component docs assume them.

- Reuse Karabiner's installed `Karabiner-DriverKit-VirtualHIDDevice` for output. No DriverKit system extension of our own, so no Apple-granted entitlement and no notarization. figaro is a personal tool; distribution to other machines is out of scope.
- The Karabiner client is pure Rust over the daemon's unix socket, not the `karabiner-driverkit` crate (which compiles a C++23 shim through `cc`). The protocol is plain bytes; keeping it in Rust keeps `freddie_virtual_hid` under `forbid(unsafe_code)`.
- Physical-key input is read as HID input values (usage page + usage + down/up), not raw input reports. Values normalize across every keyboard's report format; raw reports would need each device's report descriptor. `freddie_hid_sys` exposes values.
- The session↔daemon socket is restricted to the target user's uid (the socket carries every keystroke in both directions, so it is a keylogger and a key-injector to anyone who can open it). `hidd.md` specifies the filesystem permissions and the peer-uid check.
- The model runs in the session, never in the root daemon. This is what makes the session-side API identical to CGEventTap.

## New crates

- `freddie_hid_sys` — leaf, opts out of `forbid(unsafe_code)`, wraps `io-kit-sys`. Seize and read.
- `freddie_virtual_hid` — safe Rust, the Karabiner daemon client.
- `freddie_hid_wire` — safe Rust, the session↔daemon frame types over `freddie_keys`. Shared by `freddie_hidd` and `freddie_keyboard` (under `hid`).
- `freddie_hidd` — the root daemon binary.

## Order

Each ships and demos on its own:

1. `freddie_virtual_hid` (`hid-virtual-device-client.md`). Demo: a test binary types a string through the virtual device. Needs the Karabiner driver installed and its daemon running; needs no seize.
2. `freddie_hid_sys` (`hid-seize.md`). Demo: a root test binary seizes the keyboard and logs every key, and the keys stop reaching the system while it runs.
3. `freddie_hidd` (`hidd.md`), combining 1 and 2, plus the session socket, the LaunchDaemon plists, and the install verb. Demo: with no session client attached, the daemon can echo (post what it reads) to prove the full input-to-output loop end to end.
4. `sys/hid.rs` and the `hid` feature (`hid-session-backend.md`). Demo: a session binary calls `intercept`, remaps a key, and the remap is visible in a password field (which CGEventTap cannot do).

## Open question

Does figaro replace Karabiner-Elements, or run alongside it? They cannot coexist: Karabiner's grabber holds the exclusive seize on the physical keyboard, and figaro's HID input needs the same seize on the same device. Running figaro on HID means Karabiner-Elements' grabber is not running, which means whatever `voicemode`'s `karabiner.edn` does today either moves into figaro's model or is dropped. This does not change the keyboard mechanism in any of the component docs; it decides only whether the plan owns a migration path for those remaps, and it is why `hidd.md` takes over launching Karabiner's `VirtualHIDDevice-Daemon` (nothing else will once Karabiner-Elements is gone).

## Known platform risk

The IOKit seize path is what Apple is tightening. macOS 26.4 reportedly broke `kIOHIDOptionsTypeSeizeDevice` on the built-in MacBook keyboard specifically (Apple Developer Forums 817003); external keyboards and the general path still work, and Karabiner already had to move its output to DriverKit for adjacent reasons. Input seizing remains IOHIDManager and remains the only non-DriverKit option, but this backend is betting against a platform that is closing the door slowly.
