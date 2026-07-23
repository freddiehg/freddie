# freddie_keyboard across platforms

`freddie_keyboard` grabs the keyboard and emits keys, on macOS through `core-graphics`. Its seam is already platform-neutral: `sys/mod.rs` selects a backend, the backend speaks the `Key`/`KeyEvent`/`PressType`/`ModifierFlags` vocabulary that `freddie_keys` defines with no OS in it, and a `compile_error!` is the only thing standing where the other backends go. So this is filling in backends behind a contract that exists, not a redesign.

A cross-platform grab is what lets a freddie app that is not mercury remap keys off macOS. mercury itself stays macOS-only, because its menu bar and window observation are AppKit; the keyboard is the one piece of it that has an answer on every platform.

## The contract a backend provides

Three things, exactly what `sys/macos.rs` exports today:

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

pub struct Interceptor { /* holds the capture thread; dropping it releases the grab */ }

pub struct Emitter { /* synthesizes keys not tied to an intercepted event */ }
impl Emitter {
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError>;
    pub fn tap(&self, key: Key, flags: ModifierFlags) -> Result<(), EmitError>;
}
```

and the semantics the callback carries: `on_key` sees every key and returns what it becomes. `Some(same)` passes it, `Some(other)` remaps it, `None` drops it. The grab is exclusive: every key reaches `on_key` and nothing reaches the OS except what the backend re-emits.

Two things every backend gets from `freddie_keys` and must not reinvent:

- `Key` is abstract. Each backend owns a `Key`-to-native-code table, the shape of `sys/macos.rs`'s `TABLE`/`to_code`/`from_code`, and `Key::Raw(u16)` carries a native code the table does not name, so an unknown key round-trips instead of being lost. A key with no code on this OS is `EmitError::Unmappable`.
- `ModifierFlags` is a portable bitset. The backend maps it to the platform's native flags when it emits, and applies exactly what the event states rather than what the OS thinks is held, which is the point of stating flags on the event at all.

The `Emitter` is used from the thread that performs effects, not the capture thread, so a backend's `Emitter` must post from any thread. On all three platforms the emit primitive is thread-agnostic, so this costs nothing.

## The one hard constraint: no unsafe

The workspace sets `unsafe_code = "forbid"`, which cannot be waived with `#[allow]`. `sys/macos.rs` is unsafe-free because `core-graphics` and `core-foundation` hold the FFI and expose a safe API. Every backend must do the same: its unsafe lives in a dependency, not in `freddie_keyboard`.

- Linux has one: the `evdev` crate wraps the input ioctls and uinput and exposes a safe API, so `sys/linux.rs` is safe code over it, exactly as macOS is safe code over `core-graphics`.
- Windows has no equivalent that covers low-level hooks. The `windows` crate is unsafe at the call site. So the Windows FFI goes in a small leaf crate, `freddie_keyboard_win_sys`, that does not opt into the workspace lints and exposes a safe `install_hook`/`send_input` surface. `freddie_keyboard` depends on it and stays unsafe-free, the same shape as depending on `core-graphics`.

This keeps the `forbid` intact everywhere freddie code is written, and quarantines the unavoidable FFI in leaf crates, which is already the pattern on macOS.

## Backend selection

`sys/mod.rs`, after:

```rust
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Emitter, Interceptor, intercept};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{Emitter, Interceptor, intercept};

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(all(unix, not(target_os = "macos")))]
pub use linux::{Emitter, Interceptor, intercept};

#[cfg(not(any(target_os = "macos", target_os = "windows", all(unix, not(target_os = "macos")))))]
compile_error!("freddie_keyboard has no backend for this platform");
```

## The Windows backend

Capture is a low-level keyboard hook: `SetWindowsHookEx(WH_KEYBOARD_LL, ..)`. The hook procedure runs on the thread that installed it, and that thread must pump messages, a `GetMessage` loop, which is the exact analogue of the macOS backend's `CFRunLoop` thread. Returning non-zero from the procedure swallows the key; a remap swallows the original and injects the replacement.

Emit is `SendInput` with `INPUT_KEYBOARD` records. `ModifierFlags` become synthesized modifier down and up around the key, because Windows carries modifier state on the stream rather than on the event.

The key table is Windows virtual-key codes (`VK_*`), keyed the same way `TABLE` keys `CGKeyCode`.

`Interceptor` holds the hook thread and ends it the way `TapThread` ends the tap thread: `UnhookWindowsHookEx`, then `PostThreadMessage(WM_QUIT)` to break the message pump, then join under the same `RELEASE_TIMEOUT`.

`CaptureError` covers the refusals, which on Windows are few: a low-level hook needs no special right in a normal session, but it cannot intercept keys destined for a more-privileged window (UIPI), so a daemon that must remap inside an elevated app has to be elevated itself. That is a runtime fact to state in the error's context, not a setup step.

## The Linux backend

Capture cannot go through the display server portably. X11 has `XGrabKeyboard` and the record extension, but Wayland deliberately has no global keyboard grab, by design and for security, so nothing at the display-server layer works on both. The backend goes below the display server instead, to evdev, which is where keyd, kmonad, and keyd already live.

- Read the real keyboards at `/dev/input/event*`, take each exclusively with `EVIOCGRAB`, and read `KEY_*` events off them.
- Re-emit through a uinput virtual device: what passes `on_key` is written to uinput, which the display server reads as an ordinary keyboard, and nothing else reaches it because the real devices are grabbed.

This is display-server-agnostic: it sits under X11 and Wayland both, which is the only way to cover Wayland at all. The key table is Linux input-event codes (`KEY_A`, `KEY_ESC`, ..).

The cost is permissions. Reading `/dev/input` and writing `/dev/uinput` needs root, or membership in the `input` group plus a udev rule granting the group access to `uinput`. That is a documented setup step for whoever runs the daemon, and `CaptureError` is what a missing permission surfaces as.

Two refinements past a first version, called out so they are not mistaken for the whole job: hotplug, watching `/dev/input` for keyboards attached after start, and device selection, grabbing what has `EV_KEY` with the letter keys rather than every event device. The thread model is a blocking read or `epoll` over the device fds, and `Interceptor` drop wakes it through a self-pipe or eventfd, ungrabs, and joins.

## What is already abstracted, and what each backend fills in

- `Key`, `PressType`, `ModifierFlags`: shared, in `freddie_keys`. Each backend adds only its code table.
- The capture thread: every backend has one, and `Interceptor` is "hold a thread, drop to release" on all of them. What differs is the wakeup, a `CFRunLoop`, a Windows message pump, or an fd `epoll`, which is the backend's own business.
- The grant that a grab needs, and the reason it can be refused: Accessibility on macOS, the input group on Linux, UIPI on Windows. `CaptureError` already means "could not grab"; each backend supplies the platform's reason.

## The changes, in order

Windows and Linux are conceptually separate and independently shippable; either can land first, and each is large enough to be its own effort.

1. **`sys/mod.rs` gains the backend arms**, so a new backend is a file plus two lines rather than editing the `compile_error!`.
2. **The Windows backend.** `freddie_keyboard_win_sys` holding the hook and `SendInput` FFI behind a safe API, and `sys/windows.rs` over it with the `VK_*` table.
3. **The Linux backend.** `sys/linux.rs` on the `evdev` crate, with the `KEY_*` table, evdev grab, and uinput emit, plus the udev/input-group setup a consumer needs.
