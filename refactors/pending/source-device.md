# source-device attribution on the CGEventTap

The model learns which physical keyboard each key came from, so a binding can depend on the device (only remap the Kinesis, leave the built-in alone, and so on). This is done on the existing CGEventTap backend, with no seize, no virtual device, no root, and no Karabiner: the source device is read directly off each `CGEvent`. figaro is mercury's shape with this added; mercury calls plain `intercept` and never sees a device.

Verified on real hardware: with no seizing remapper running, a listen tap resolves `Kinesis Adv360` and `Apple Internal Keyboard / Trackpad` as distinct devices straight from the events. The technique ships in LinearMouse (notarized, no entitlement, hardened runtime).

## The mechanism

Each `CGEvent` from a hardware key wraps the kernel `IOHIDEvent` it came from, and that event names its source device. Two private symbols reach it, plus public IOKit to resolve the id to a device:

```c
IOHIDEventRef CGEventCopyIOHIDEvent(CGEventRef);   // CoreGraphics, private; NULL for synthetic events
uint64_t      IOHIDEventGetSenderID(IOHIDEventRef); // IOKit SPI; the registry entry id of the source HID service
```

`senderID` is an IOKit registry entry id. `IORegistryEntryIDMatching(senderID)` → `IOServiceGetMatchingService` → walk parents to the `IOHIDDevice` → read `VendorID`, `ProductID`, `Product`, `Built-In`. This resolution path is the one the spike used and it returned the right names.

Injected and software-synthesized events (including untagged posts from other apps) have no HID backing: `CGEventCopyIOHIDEvent` returns `NULL` or the sender id is `0`. So a real-hardware key always carries a source; a synthetic one never does. That is the split the model wants, and it comes for free. Own emits are tagged by the interceptor and never reach the callback, so the consumer does not classify them.

Two keyboards at once needs no handling: each `CGEvent` independently carries its own source, so simultaneous keys on two devices are attributed separately with nothing to correlate.

## The one requirement

Nothing may be seizing the keyboard upstream. A seizing remapper (Karabiner-Elements' grabber) reads the physical device and re-emits through its own virtual keyboard, so by the time a session tap sees a key, its source is that virtual device, not the real one. The spike showed this directly: with Karabiner running, every key resolved to "Karabiner DriverKit VirtualHIDKeyboard"; with it quit, the real devices appeared. So figaro is the sole grabber, which is the standing decision that figaro replaces Karabiner-Elements.

## The leaf crate

The two private symbols are not in the `core-graphics`/`io-kit-sys` safe wrappers, so the FFI is quarantined in a leaf crate, `freddie_hid_device` (same planned shape as `freddie_keyboard_win_sys`: its own lint table, not workspace `forbid`). Everything above it stays under `forbid(unsafe_code)`.

`SourceId` lives in `freddie_hid_device` and is the cache key for resolve. Consumers of `intercept_with_source` never see it: they get a resolved `DeviceInfo`. `KeyEvent` does not carry either, so `freddie_keys` never learns they exist.

```rust
// crates/freddie_hid_device/src/lib.rs  — own lint table (unsafe_code = deny), FFI in a private module.

/// A source device's identity for the run: the IOKit registry entry id of the originating HID
/// service. Stable while the device stays attached; a replug yields a new one. Internal to the
/// leaf and to `freddie_keyboard`'s resolve cache; not part of the consumer callback.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SourceId(pub u64);

// The two private symbols, forward-declared: neither is in a public -sys crate.
unsafe extern "C" {
    fn CGEventCopyIOHIDEvent(event: CGEventRef) -> IOHIDEventRef; // CoreGraphics; +1, or null for synthetic
    fn IOHIDEventGetSenderID(event: IOHIDEventRef) -> u64;        // IOKit SPI
}

/// The source HID service of a `CGEvent`, or `None` for an injected/synthetic event. Two calls and
/// a release, no allocation, no registry walk — cheap enough for the tap thread.
pub fn source_of(event: CGEventRef) -> Option<SourceId> {
    // SAFETY: the private symbol returns a +1 IOHIDEvent, or null when the event has no HID origin.
    let hid = unsafe { CGEventCopyIOHIDEvent(event) };
    if hid.is_null() {
        return None;
    }
    // SAFETY: `hid` is a live +1 event; read its sender, then drop our reference.
    let sender = unsafe { IOHIDEventGetSenderID(hid) };
    unsafe { CFRelease(hid.cast()) };
    (sender != 0).then_some(SourceId(sender))
}

/// What a source resolves to. Each field comes off the service entry the id names, or the nearest
/// ancestor that carries it (some live on the parent `IOHIDDevice`, not the service).
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub built_in: bool,
}

/// The device behind a `SourceId`. One registry lookup. `None` if the id matches no service
/// (a keyboard unplugged since the key was pressed). Callers that hit this per key cache by
/// `SourceId` — `intercept_with_source` does.
pub fn resolve(id: SourceId) -> Option<DeviceInfo> {
    // SAFETY: IORegistryEntryIDMatching returns a +1 dict; IOServiceGetMatchingService consumes it
    // and hands back a +1 service, or 0 (MACH_PORT_NULL) when nothing matches.
    let service = unsafe {
        IOServiceGetMatchingService(kIOMainPortDefault, IORegistryEntryIDMatching(id.0))
    };
    if service == 0 {
        return None;
    }
    let info = DeviceInfo {
        vendor_id: prop_u32(service, "VendorID").unwrap_or(0) as u16,
        product_id: prop_u32(service, "ProductID").unwrap_or(0) as u16,
        product: prop_string(service, "Product").unwrap_or_default(),
        built_in: prop_bool(service, "Built-In").unwrap_or(false),
    };
    // SAFETY: release the service we were handed.
    unsafe { IOObjectRelease(service) };
    Some(info)
}

/// Read `key` off `entry`, or the nearest ancestor that has it, walking
/// `IORegistryEntryGetParentEntry` up the service plane (`IORegistryEntryCreateCFProperty` at each
/// step, checking the CF type). The types differ and it matters: `Built-In` is a CFBoolean,
/// `VendorID`/`ProductID` are CFNumbers, `Product` a CFString. The spike read `Built-In` as a
/// number and always got `?`, which is why `prop_bool` reads it as a CFBoolean here.
fn prop_bool(entry: io_registry_entry_t, key: &str) -> Option<bool>;
fn prop_u32(entry: io_registry_entry_t, key: &str) -> Option<u32>;
fn prop_string(entry: io_registry_entry_t, key: &str) -> Option<String>;
```

`CGEventCopyIOHIDEvent` and `IOHIDEventGetSenderID` are the only private symbols; everything `resolve` calls is public IOKit (`IORegistryEntryIDMatching`, `IOServiceGetMatchingService`, `IORegistryEntryCreateCFProperty`, `IORegistryEntryGetParentEntry`, `IOObjectRelease`). That is the whole crate: stamp an id onto the event, and turn an id into a `DeviceInfo`. Deciding which device is which is the consumer's, and it is small.

## The device-aware entry point

`KeyEvent` does not change, and neither does `freddie_keys`. The device rides alongside the key, not inside it: `freddie_keyboard` exposes two streams over its one internal tap. That tap's callback closes over the `CGEventRef`, and nothing above `freddie_keyboard` ever sees it, so only a stream built here can read the source. `intercept` is the stream without it; `intercept_with_source` is the same stream, resolving the source to a `DeviceInfo` per key.

```rust
// freddie_keyboard, macOS. `intercept` is unchanged; what mercury uses.
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

// The same tap, plus the resolved device of each key. What figaro uses.
// One argument: the key paired with its device (`None` = synthetic / no HID origin).
pub fn intercept_with_source(
    on_key: impl Fn((KeyEvent, Option<DeviceInfo>)) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;
```

Both are thin entry points over the same tap. `intercept_with_source` owns a `HashMap<SourceId, DeviceInfo>` on the tap thread (not shared across threads): for each key it runs `source_of`, then a cache hit or one `resolve`, and hands `(key, info)` to the callback. First sight of a device is one registry walk; every later key is a hash hit and a `DeviceInfo` clone. A consumer that wants nothing to do with the device calls `intercept` and never triggers the source read.

```rust
// inside the tap callback for intercept_with_source (sketch):
let device = source_of(event.as_ptr()).and_then(|id| {
    if let Some(cached) = cache.get(&id) {
        return Some(cached.clone());
    }
    let info = resolve(id)?;
    cache.insert(id, info.clone());
    Some(info)
});
match decide(&input, on_key((input.clone(), device))) { /* Pass / Drop / Remap */ }
```

No feature gates this. `freddie_keyboard` always depends on `freddie_hid_device` and re-exports `DeviceInfo` (and `SourceId` / `source_of` / `resolve` only for demos and tests that need the lower layer). The private symbol lives in CoreGraphics, which the crate already links, and it has been stable for over a decade and ships in notarized apps, so guarding mercury against its disappearance is not worth a build flag. mercury just calls `intercept`.

figaro never calls `resolve` and never sees a `SourceId`. Classification is pure over `DeviceInfo` at its boundary. The consumer-side design is `figaro/refactors/pending/device-conditioned-keymaps.md`.

```rust
// figaro, at the boundary. DeviceInfo is freddie's; the class is figaro's.
enum DeviceClass { Desktop, Laptop, Other, Injected }

fn classify(device: Option<&DeviceInfo>) -> DeviceClass {
    match device {
        None => DeviceClass::Injected,
        Some(d) if d.built_in => DeviceClass::Laptop,
        Some(d) if d.vendor_id == 0x29ea && d.product_id == 0x0360 => DeviceClass::Desktop, // Kinesis Adv360
        Some(_) => DeviceClass::Other,
    }
}
```

## Cost, stated plainly

- No remapping inside secure input (password fields): the CGEventTap is bypassed there, so figaro does not see or remap those keys. This is the one thing the HID route would have bought and this does not; it is out of scope here (see `hid-backend.md`, the deferred secure-input upgrade).
- The two device symbols are private and undocumented. They are stable across years and ship in notarized apps, but a future macOS could change them. Every call guards for `NULL`/`0` and degrades to `device: None` rather than failing, so a break costs device-awareness, not the remapper.
- First key from a newly seen device does a registry walk on the tap thread. That is once per attachment per process; later keys are a map lookup. Two keyboards at boot means two walks, not a walk per keystroke.

## Tests

- `prop_bool`/`prop_u32`/`prop_string` read the right CF types, and walk to a parent when the entry itself lacks the key (the built-in keyboard's `Built-In` lives up the plane). This is where the spike's `Built-In` = `?` gets fixed, so it is worth a real test against a known service.
- `source_of` returns `None` for a synthetic event (built with `CGEvent::new_keyboard_event`, no HID origin) and `Some` for the hardware path, which is the manual spike, not a unit test.
- The resolve cache: a second key with the same `SourceId` does not call `resolve` again (unit-testable with a thin seam or by counting through a test double if one is introduced; otherwise covered by the demo).

The end-to-end proof is the spike in this session: two physical keyboards resolved by name off the tap, and it is the acceptance test for the leaf crate — run it, type on two keyboards, see two devices.
