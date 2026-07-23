# freddie_hid_sys: seizing and reading the physical keyboard

The leaf crate that takes exclusive control of the physical keyboards and delivers each key as it is pressed and released. This is the input half of the HID backend and the only place in the tree that writes `unsafe`. It wraps `io-kit-sys` and exposes a safe callback API in `freddie_keys`' vocabulary is not its concern: it speaks raw HID usages, and `freddie_hidd` maps those to `Key`.

Verified against `io-kit-sys` 0.5.0, Apple `IOHIDManager.h`/`IOHIDDevice.h`/`IOHIDKeys.h`, TN2187, and Karabiner's `DEVELOPMENT.md`.

## Crate

```toml
# crates/freddie_hid_sys/Cargo.toml
[package]
name = "freddie_hid_sys"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
io-kit-sys = "0.5"
core-foundation = { version = "0.10", features = ["link"] }
core-foundation-sys = "0.8"
tracing = "0.1"
```

This crate does not take `[lints] workspace = true`, because the workspace forbids `unsafe` and this crate needs it. It sets its own lints and scopes the unsafe:

```rust
// crates/freddie_hid_sys/src/lib.rs
#![cfg_attr(not(test), deny(clippy::all))]
// Deliberately NOT forbid(unsafe_code): this is the FFI quarantine the workspace's
// forbid depends on. The unsafe lives in `sys`; the public API is safe.
```

Everything unsafe is in a private `sys` module. The public surface is a `Keyboards` handle plus a value callback, and nothing above it in the tree gains the ability to write unsafe.

## What it delivers

Physical keys come as HID input values, not raw reports. A value callback fires once per element transition with a usage page, a usage id, and an integer value (1 down, 0 up), already normalized across whatever report format the device uses. Raw reports would force us to parse each keyboard's report descriptor and handle report ids; values do not.

```rust
/// One physical key transition from a seized keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HidInput {
    /// HID usage page. Keyboard keys are 0x07 (Keyboard/Keypad); some keys
    /// (power, some media) arrive on 0x01/0x0C, passed through so the mapper decides.
    pub usage_page: u16,
    /// HID usage id within the page.
    pub usage: u16,
    /// Whether the key went down or came up.
    pub press: freddie_keys::PressType,
    /// Which physical device it came from, so the mapper can tell keyboards apart.
    pub device: DeviceId,
}

/// A stable per-device handle for the life of a seize. A seized device that is unplugged
/// and replugged gets a new one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DeviceId(u64);
```

`PressType` is reused from `freddie_keys` so the daemon does not translate a bool. The usage id is left raw: mapping usage → `Key` belongs to `freddie_hidd`, which owns the table, exactly as `sys/macos.rs` owns the `CGKeyCode` table.

## The public API

```rust
/// An active seize of every matching keyboard. While it is alive, their keys do not reach
/// the system and arrive at the callback instead. Dropping it releases every device and
/// stops the run loop.
pub struct Keyboards { /* run-loop thread, manager, callback box */ }

/// Why the seize could not start.
pub enum SeizeError {
    /// Input Monitoring is not granted to this binary, or the process is not root.
    /// Both are required; neither is distinguishable from the other at this layer.
    Denied,
    /// IOKit refused to create the manager or open a device for a reason other than a
    /// missing grant.
    IoKit(i32),   // the IOReturn
}

/// Seize all keyboards (Generic Desktop / Keyboard) and call `on_input` for each key.
///
/// `on_input` runs on the seize thread's run loop, so it must not block: send on a channel
/// and return, the way the daemon does. It is `Fn`, `Send`, `'static`.
///
/// Requires root and Input Monitoring. See the crate docs.
pub fn seize(
    on_input: impl Fn(HidInput) + Send + 'static,
) -> Result<Keyboards, SeizeError>;
```

The shape mirrors `freddie_keyboard::intercept`: a callback in, a drop-to-release handle out, a dedicated run-loop thread inside. `Keyboards::drop` stops the `CFRunLoop` and joins under a bounded timeout, the way `TapThread` does in `sys/macos.rs`, so one wedged callback cannot make the daemon unkillable.

## Inside `sys` (the unsafe)

The FFI sequence, all inside `mod sys`:

1. `IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone)`.
2. Build the matching dictionary and set it:

```rust
// kIOHIDDeviceUsagePageKey = "DeviceUsagePage", kIOHIDDeviceUsageKey = "DeviceUsage".
// Prefer these over the legacy kIOHIDPrimaryUsage* keys: a composite keyboard+mouse
// reports only its primary usage under the old keys and would be missed.
let matching = cfdict! {
    "DeviceUsagePage" => 0x01,  // Generic Desktop
    "DeviceUsage"     => 0x06,  // Keyboard
};
IOHIDManagerSetDeviceMatching(manager, matching.as_concrete_TypeRef());
```

3. Register the value callback and (for device ids and hotplug) the matching/removal callbacks:

```rust
IOHIDManagerRegisterInputValueCallback(manager, value_trampoline, context);
IOHIDManagerRegisterDeviceMatchingCallback(manager, matched_trampoline, context);
IOHIDManagerRegisterDeviceRemovalCallback(manager, removed_trampoline, context);
```

4. Schedule on this thread's run loop, then open with the seize option, then run the loop:

```rust
IOHIDManagerScheduleWithRunLoop(manager, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
let rv = IOHIDManagerOpen(manager, kIOHIDOptionsTypeSeizeDevice); // 0x01
if rv != kIOReturnSuccess { /* map to SeizeError, Denied on the privilege codes */ }
CFRunLoopRun();
```

`kIOHIDOptionsTypeSeizeDevice` is `0x01`. Scheduling propagates to devices matched later, so hotplugged keyboards are seized without re-opening the manager.

### The trampoline and the context

The callback is a C function pointer with a `void* context`. The context is a `Box`ed struct holding the safe `on_input` closure and a `DeviceId` map; it is created before `CFRunLoopRun`, passed as `*mut c_void`, and reclaimed when the manager is torn down. The trampoline does the minimum unsafe work and hands off to safe code immediately:

```rust
extern "C" fn value_trampoline(
    context: *mut c_void, _result: IOReturn, sender: *mut c_void, value: IOHIDValueRef,
) {
    // SAFETY: context is the Box we passed to Register*, alive until the run loop stops.
    let ctx = unsafe { &*(context as *const Context) };
    // SAFETY: value/sender are valid for the duration of the callback per IOKit contract.
    let element = unsafe { IOHIDValueGetElement(value) };
    let usage_page = unsafe { IOHIDElementGetUsagePage(element) } as u16;
    let usage = unsafe { IOHIDElementGetUsage(element) } as u16;
    let pressed = unsafe { IOHIDValueGetIntegerValue(value) } != 0;
    let device = ctx.device_id(sender);
    // Everything past here is safe.
    if let Some(input) = ctx.classify(usage_page, usage, pressed, device) {
        (ctx.on_input)(input);
    }
}
```

`classify` drops the elements that are not keys (a keyboard element collection reports array-index and modifier elements; only usage transitions with a nonzero usage are keys) and builds `HidInput`. It is pure and unit-tested. The `unsafe` is confined to the four `IOHID*Get*` reads and the context deref; the rest is ordinary Rust.

### Teardown

`Keyboards` holds the `CFRunLoop` and the thread. Drop stops the loop (`CFRunLoopStop` is thread-safe), the run-loop thread returns from `CFRunLoopRun`, `IOHIDManagerClose(manager, kIOHIDOptionsTypeSeizeDevice)` releases every device, and the `Context` box is dropped. Bounded join, as in `sys/macos.rs`.

## Permissions, surfaced not solved

`seize` returns `SeizeError::Denied` when the open fails for want of root or Input Monitoring. This crate does not prompt or request; it reports. `freddie_hidd` decides what to do (log where to grant it). Driving the TCC prompt (`IOHIDRequestAccess`/`IOHIDCheckAccess`) needs a user session and belongs to the session side, covered in `hidd.md`.

## Tests

- `classify` maps a Keyboard-page usage transition to `HidInput` with the right `PressType`, and drops non-key elements (array selectors, the constant reserved element).
- The `DeviceId` map assigns stable ids per `sender` pointer and a fresh id after a removal+rematch.
- The usage constants used by `classify` match `io-kit-sys`' `usage_tables` (a compile-time cross-check, not hand-copied numbers).

The seize itself is a manual root demo: run it, confirm keys stop reaching the foreground app and appear in the log, drop it, confirm the keyboard returns to normal.
