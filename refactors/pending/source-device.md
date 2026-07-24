# source-device attribution on the CGEventTap

The model learns which physical keyboard each key came from, so a binding can depend on the device (only remap the Kinesis, leave the built-in alone, and so on). This is done on the existing CGEventTap backend, with no seize, no virtual device, no root, and no Karabiner: the source device is read directly off each `CGEvent`. figaro is mercury's shape with this added; mercury gets the field and ignores it.

Verified on real hardware: with no seizing remapper running, a listen tap resolves `Kinesis Adv360` and `Apple Internal Keyboard / Trackpad` as distinct devices straight from the events. The technique ships in LinearMouse (notarized, no entitlement, hardened runtime).

## The mechanism

Each `CGEvent` from a hardware key wraps the kernel `IOHIDEvent` it came from, and that event names its source device. Two private symbols reach it, plus public IOKit to resolve the id to a device:

```c
IOHIDEventRef CGEventCopyIOHIDEvent(CGEventRef);   // CoreGraphics, private; NULL for synthetic events
uint64_t      IOHIDEventGetSenderID(IOHIDEventRef); // IOKit SPI; the registry entry id of the source HID service
```

`senderID` is an IOKit registry entry id. `IORegistryEntryIDMatching(senderID)` → `IOServiceGetMatchingService` → walk parents to the `IOHIDDevice` → read `VendorID`, `ProductID`, `Product`, `Built-In`. This resolution path is the one the spike used and it returned the right names.

Injected and software-synthesized events (including our own re-emitted output, and anything another app posts) have no HID backing: `CGEventCopyIOHIDEvent` returns `NULL` or the sender id is `0`. So a real-hardware key always carries a source; a synthetic one never does. That is the split the model wants, and it comes for free.

Two keyboards at once needs no handling: each `CGEvent` independently carries its own source, so simultaneous keys on two devices are attributed separately with nothing to correlate.

## The one requirement

Nothing may be seizing the keyboard upstream. A seizing remapper (Karabiner-Elements' grabber) reads the physical device and re-emits through its own virtual keyboard, so by the time a session tap sees a key, its source is that virtual device, not the real one. The spike showed this directly: with Karabiner running, every key resolved to "Karabiner DriverKit VirtualHIDKeyboard"; with it quit, the real devices appeared. So figaro is the sole grabber, which is the standing decision that figaro replaces Karabiner-Elements.

## The leaf crate

The two private symbols are not in the `core-graphics`/`io-kit-sys` safe wrappers, so the FFI is quarantined in a leaf crate, `freddie_hid_device`, the pattern the workspace already uses for `unsafe`. Everything above it stays under `forbid(unsafe_code)`.

```rust
// crates/freddie_hid_device/src/lib.rs  — opts out of forbid(unsafe_code), FFI in a private module.

/// A source device's identity for the run: the IOKit registry entry id of the originating HID
/// service. Stable while the device stays attached; a replug yields a new one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SourceId(pub u64);

/// The source HID service of a `CGEvent`, or `None` for an injected/synthetic event (no HID
/// backing, or sender id 0). `event` is the raw `CGEventRef` the tap callback already holds,
/// from `core_graphics`' `CGEvent`.
pub fn source_of(event: CGEventRef) -> Option<SourceId>;

/// What a source resolves to. Read from the `IOHIDDevice` behind the service.
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub source: SourceId,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub built_in: bool,
}

/// Resolves a `SourceId` to its `DeviceInfo`, caching per id. First sight of an id does the
/// registry walk; after that it is a hash lookup. A replugged keyboard is a new id and resolves
/// fresh, so no hotplug watcher is needed.
pub struct Devices { /* HashMap<SourceId, Option<DeviceInfo>> */ }
impl Devices {
    pub fn new() -> Self;
    pub fn resolve(&mut self, id: SourceId) -> Option<&DeviceInfo>;
}
```

`source_of` is the only place the private symbols are called; `Devices::resolve` is public IOKit (`IORegistryEntryIDMatching`, `IOServiceGetMatchingService`, `IORegistryEntryCreateCFProperty`, `IORegistryEntryGetParentEntry`, `IOObjectRelease`) plus CF property reads. Each call guards for a failed copy and for sender id 0, and yields `None` rather than a wrong device.

## The change to `freddie_keys`

`KeyEvent` gains the source, `None` when the key was not real hardware:

```rust
// before
pub struct KeyEvent { pub key: Key, pub press: PressType, pub flags: ModifierFlags }
// after
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags,
    /// Which physical device produced this key, or `None` for an injected/synthetic event.
    pub device: Option<freddie_hid_device::SourceId>,
}
```

`SourceId` is a plain `Copy` id, so it costs nothing on the event. The rich `DeviceInfo` (name, vendor, product, built-in) is what config is written against, and the model resolves it from the id through `Devices` at the point it needs it (building a per-device binding table keyed by vendor:product, or matching a rule), not on every keystroke. The Debug impl omits `device` when `None`, as it already does for empty `flags`.

## The change to `freddie_keyboard`

`sys/macos.rs`'s tap callback already builds the incoming `KeyEvent`. It stamps the source:

```rust
// before
let input = KeyEvent { key: from_code(code), press, flags: from_cg(event.get_flags()) };
// after
let input = KeyEvent {
    key: from_code(code),
    press,
    flags: from_cg(event.get_flags()),
    device: freddie_hid_device::source_of(event.as_ptr()),
};
```

The `Emitter` fills `device: None` on anything it constructs, since emitted keys are synthetic. `intercept`'s signature does not change; the device rides on the `KeyEvent` the callback already delivers. mercury reads none of it and is unaffected.

`freddie_keyboard` re-exports `freddie_hid_device::{SourceId, DeviceInfo, Devices}` so a consumer resolves a source without naming the leaf crate directly.

## What figaro does with it

figaro depends on `freddie_keyboard` (the CGEventTap backend), exactly like mercury — there is no separate keyboard crate. It holds a `Devices` and, when a binding cares about origin, resolves `event.device` to a `DeviceInfo` and matches it against its config. A key with `device: None` is injected, not from a keyboard the user pressed, and figaro treats it as such.

## Cost, stated plainly

- No remapping inside secure input (password fields): the CGEventTap is bypassed there, so figaro does not see or remap those keys. This is the one thing the HID route would have bought and this does not; it is out of scope here (see `hid-backend.md`, the deferred secure-input upgrade).
- The two device symbols are private and undocumented. They are stable across years and ship in notarized apps, but a future macOS could change them. Every call guards for `NULL`/`0` and degrades to `device: None` rather than failing, so a break costs device-awareness, not the remapper.

## Tests

- `source_of` returns `None` for a synthetic event (one built with `CGEvent::new_keyboard_event` and posted with no HID origin) and `Some` for... the hardware path, which is the manual spike, not a unit test.
- `Devices::resolve` caches: a second lookup of the same id does no registry call (inject a counting fake behind the registry calls, or assert via a public hit-count in test builds).
- A `KeyEvent` with `device: None` renders without the field under `Debug`, matching the `flags` treatment.

The end-to-end proof is the spike in this session: two physical keyboards resolved by name off the tap, and it is the acceptance test for the leaf crate — run it, type on two keyboards, see two devices.
