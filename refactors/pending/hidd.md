# freddie_hidd: the root HID daemon

The root `LaunchDaemon` that wires the seize (`freddie_hid_sys`) to the virtual device (`freddie_virtual_hid`) and exposes a unix socket to the session. It runs the transport, not the model: it reads the physical keyboard, forwards each key to the session, applies the keys the session emits back, and posts them to the virtual device. When no session client is attached it passes keys straight through, so the keyboard never goes dead.

Depends on `hid-virtual-device-client.md`, `hid-seize.md`, and `freddie_hid_wire` (the session↔daemon protocol, defined in `hid-session-backend.md`: `Uplink::Input(KeyEvent)` up, `Downlink::Emit { key, press }` down, `u32`-length-prefixed JSON frames). This is the change that ties the pieces together; once it lands, `freddie_keyboard_hid` from the first change drives it end to end.

## The daemon's state

Two independent halves, an input side and an output side, each a small state machine.

Input side, so an uplinked `KeyEvent` carries flags like the tap does:

```rust
/// Which modifier keys are physically held, tracked from the seized stream so a
/// non-modifier key can be stamped with the modifiers held with it. Mirrors what
/// `sys/macos.rs` reads off a CGEvent's flags.
struct InputModifiers(freddie_keys::ModifierFlags);
```

Output side, the absolute-state the virtual device needs:

```rust
/// What the virtual keyboard currently holds. The daemon toggles this per downlinked
/// `Emit` and re-posts the whole state (the wire to Karabiner is absolute, not events).
/// This is `freddie_virtual_hid`'s `KeyboardState` behind the daemon's own key vocabulary.
struct Output {
    keyboard: freddie_virtual_hid::VirtualKeyboard,
    held: HeldKeys,   // the current set of freddie HidKeys, re-serialized on each change
}
```

## The usage ↔ Key table

The daemon owns the map between HID Keyboard/Keypad usages and `freddie_keys::Key`, the HID analogue of `TABLE` in `sys/macos.rs`. One row per named key; a usage the table does not name becomes `Key::Raw(usage)`, and a `Key::Raw(u16)` emitted by the model is posted as that usage verbatim.

```rust
/// Every named key and its HID Keyboard/Keypad (page 0x07) usage id.
const TABLE: &[(Key, u16)] = &[
    (Key::KeyA, 0x04),
    (Key::KeyB, 0x05),
    // ... e=0x08, Return=0x28, Escape=0x29, Space=0x2C, Tab=0x2B, ...
    (Key::ShiftLeft, 0xE1),
    (Key::ControlLeft, 0xE0),
    (Key::AltLeft, 0xE2),
    (Key::MetaLeft, 0xE3),
    (Key::ShiftRight, 0xE5),
    (Key::ControlRight, 0xE4),
    (Key::AltRight, 0xE6),
    (Key::MetaRight, 0xE7),
    // ... the full set, one row per Key variant that has a usage
];

/// Usage → Key for the uplink. Unnamed usages become `Key::Raw(usage)`.
fn key_from_usage(page: u16, usage: u16) -> Option<Key>;   // None for non-keyboard pages

/// Key → the virtual device's `HidKey` for the downlink. A modifier Key maps to its sided
/// modifier bit; every other Key maps to a `Usage`. `Key::Raw(u16)` is that usage.
fn hid_key_of(key: Key) -> Option<freddie_virtual_hid::HidKey>;   // None: no usage on HID
```

Modifiers map to the sided Karabiner bit directly from the `Key` (which is already sided: `MetaLeft` vs `MetaRight`), so the output modifier state is exact and never guessed from an unsided flag.

## The two loops

The seize callback and the socket reader each feed a single owner thread that mutates the daemon state, so there is one writer and no lock on the state, the pattern mercury's worker uses. `select!` over the two sources, never a poll.

Input (seize → session, or → passthrough):

```
on each HidInput from freddie_hid_sys:
    key = key_from_usage(input.usage_page, input.usage)   // skip if None
    update InputModifiers if key is a modifier
    event = KeyEvent { key, press: input.press, flags: input_modifiers }
    if a session client is connected:
        send Uplink::Input(event)          // the session decides the output
    else:
        apply event to Output.held; post   // passthrough: keyboard stays alive
```

Output (session → virtual device):

```
on each Downlink::Emit { key, press } from the session:
    hid = hid_key_of(key)                  // warn and drop if None (no usage on HID)
    if press == Down { Output.held.insert(hid) } else { Output.held.remove(hid) }
    Output.keyboard.set_state(Output.held.as_slice())
```

When a client is connected the daemon does not passthrough; the session owns all output. On client disconnect the daemon reverts to passthrough and clears any half-held emitted state. This is the safety property: a figaro that crashes or was never started leaves a working keyboard, not a seized-dead one.

## The daemon's own socket

```rust
/// Where the daemon binds for the session. Under a directory it creates and locks to the
/// target user, because every keystroke crosses this socket in both directions: anyone who
/// can open it can read and inject keys.
const SOCKET_DIR: &str = "/Library/Application Support/hg.freddie";
const SOCKET_NAME: &str = "hidd.sock";
```

The daemon runs as root and the session agent as the user, so the socket cannot be root-only. It is locked to one uid, passed to the daemon at install as `--user <uid>`:

- The directory is created `0700` and `chown`ed to the target uid; the socket file is `chmod 0600` and `chown`ed to it. Filesystem permissions are the primary gate, as they are for Karabiner's own socket.
- Every accepted connection is checked with `getpeereid(2)`: the peer's euid must equal the target uid, else the connection is dropped. `getpeereid` is the one unsafe call on this path, so it lives in `freddie_hid_sys` (already the FFI leaf) behind a safe `peer_uid(&UnixStream) -> io::Result<u32>`, rather than opening a second unsafe crate.

One client at a time: a second connection is refused while one is live. The session is a singleton per user already (`freddie_single_instance`), so a second connection means something is wrong, and refusing it keeps the keystroke stream unambiguous.

## Running as a root LaunchDaemon

The daemon is a system singleton, not a per-user agent, so its lifecycle is distinct from mercury's `LaunchAgent` (`crates/mercury/src/agent.rs`):

- Domain `system`, not `gui/<uid>`. Installed with `sudo launchctl bootstrap system /Library/LaunchDaemons/hg.freddie.hidd.plist`.
- The plist runs the daemon binary with `--user <console-uid>`, `RunAtLoad`, and `KeepAlive` only on unclean exit (`SuccessfulExit=false`), with a `ThrottleInterval` so a crash loop cannot seize the keyboard ten times a second, mirroring the agent's `KeepAlive`.
- The single-instance lock is system-wide: `freddie_single_instance` keyed to a path under `/Library/Application Support/hg.freddie`, not the per-user directory it uses today. This is a new `acquire`-at path, and the crate already takes the lock path as an argument (`holder_at`/`acquire_at`), so no change to it beyond the caller passing a system path.
- Logs go to `/Library/Logs/hg.freddie/hidd.log` (root-writable, system-wide), not `~/Library/Logs`. `freddie_cli`'s logging takes the directory; the daemon passes the system one.

Karabiner's `Karabiner-VirtualHIDDevice-Daemon` is not auto-started once Karabiner-Elements is not running (the pkg ships no persistent plist for it). The install writes a second `LaunchDaemon`, `hg.freddie.karabiner-vhidd.plist`, that runs the installed daemon binary with `ProcessType Interactive`, so the virtual-device socket exists whenever `freddie_hidd` needs it.

## The install verb

A verb distinct from mercury's `install`, because it needs root and the `system` domain. It:

1. Resolves the console user's uid (the session agent's uid).
2. Writes `hg.freddie.hidd.plist` (running this binary `daemon --user <uid>`) and `hg.freddie.karabiner-vhidd.plist` into `/Library/LaunchDaemons`, serialized through `plist` the way the agent is, so a path with an `&` cannot produce malformed XML.
3. `launchctl bootout system/...` then `bootstrap system ...` both, idempotently.
4. Prints where to grant Input Monitoring, because the grant cannot be scripted.

It requires `sudo`; run without root it exits with that instruction rather than a launchctl error.

## Input Monitoring

The seize returns `SeizeError::Denied` until the daemon binary is granted Input Monitoring, and a root daemon at the login window cannot raise the TCC prompt itself. The grant is driven from the session side: the session agent, which has an Aqua session, calls `IOHIDRequestAccess(kIOHIDRequestTypeListenEvent)` to raise the prompt, and the daemon binary is what appears in System Settings > Privacy & Security > Input Monitoring. Until it is granted, the daemon logs the exact pane to open and stays in passthrough by not seizing (so the keyboard still works, unremapped). The session-side `IOHIDRequestAccess` call is unsafe FFI and lives in `freddie_hid_sys` behind a safe `request_input_monitoring()`.

A dev binary under `target/` gets its Input Monitoring grant keyed to its path and loses it whenever the identity changes; `cargo install` to a stable path and re-grant, the same caveat mercury's agent prints for `target/`.

## Tests

- `key_from_usage`/`hid_key_of` round-trip named keys and pass unnamed usages through `Key::Raw`, the same table tests `sys/macos.rs` has.
- The input loop stamps the tracked `InputModifiers` onto a non-modifier `KeyEvent` and updates them on a modifier transition.
- The output loop toggles `held` on `Emit` and posts a state whose bytes match the expected report (leaning on `freddie_virtual_hid`'s `report_bytes`).
- Passthrough: with no client, an input key becomes an output post of the same key; with a client, it becomes an `Uplink::Input` and no post.
- Peer-uid: a connection whose `getpeereid` differs from the target is dropped before any frame is read.

End to end (seize a key, see it typed through the virtual device with no session client) is the manual demo that proves the daemon before the session backend exists.
