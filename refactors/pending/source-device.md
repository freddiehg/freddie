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

`SourceId` lives in `freddie_hid_device`, next to the code that produces it. `KeyEvent` does not carry it, so `freddie_keys` never learns it exists.

```rust
// crates/freddie_hid_device/src/lib.rs  — opts out of forbid(unsafe_code), FFI in a private module.

/// A source device's identity for the run: the IOKit registry entry id of the originating HID
/// service. Stable while the device stays attached; a replug yields a new one.
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

/// The device behind a `SourceId`. Stateless: one registry lookup, the caller caches the result.
/// `None` if the id matches no service (a keyboard unplugged since the key was pressed).
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

`KeyEvent` does not change, and neither does `freddie_keys`. The source rides alongside the key, not inside it: `freddie_keyboard` exposes two streams over its one internal tap. That tap's callback closes over the `CGEventRef`, and nothing above `freddie_keyboard` ever sees it, so only a stream built here can read the source. `intercept` is the stream without it; `intercept_with_source` is the same stream, closing over the `CGEventRef` to compute the source per key.

```rust
// freddie_keyboard, macOS. `intercept` is unchanged; what mercury uses.
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

// The same tap, plus the source id of each key. What figaro uses.
pub fn intercept_with_source(
    on_key: impl Fn(KeyEvent, Option<SourceId>) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;
```

Both are thin entry points over the same tap; `intercept_with_source`'s callback additionally computes `source_of(event.as_ptr())` (two calls and a release, on the tap thread) and hands it over as the second argument. A consumer that wants nothing to do with the device calls `intercept` and never triggers the source read.

No feature gates this. `freddie_keyboard` always depends on `freddie_hid_device` (the unsafe leaf) and re-exports `SourceId`, `resolve`, and `DeviceInfo`, so a consumer names one crate. The private symbol lives in CoreGraphics, which the crate already links, and it has been stable for over a decade and ships in notarized apps, so guarding mercury against its disappearance is not worth a build flag. mercury just calls `intercept`; the other entry point exists and it ignores it.

The read on the tap thread is the cheap half; resolving an id to a class is the costly half, and it does not happen here. It happens where figaro classifies, off the tap thread and cached (below).

## What figaro does with it

figaro calls `intercept_with_source` and gets a `(KeyEvent, Option<SourceId>)` per key. At its boundary (the worker, off the tap thread) it turns the source id into its own class and caches it, so the registry walk happens once per device and never in a handler. There is no shared classifier: figaro cares about two keyboards, so its whole policy is a few lines over `DeviceInfo`. The consumer-side design is `figaro/refactors/pending/device-conditioned-keymaps.md`.

```rust
// figaro, at the boundary. `resolve` and `DeviceInfo` are freddie's; the rest is figaro's.
enum DeviceClass { Desktop, Laptop, Other, Injected }

fn classify(id: SourceId) -> DeviceClass {
    match resolve(id) {
        Some(d) if d.built_in => DeviceClass::Laptop, // a bool, so test it first
        Some(d) if d.vendor_id == 0x29ea && d.product_id == 0x0360 => DeviceClass::Desktop, // Kinesis Adv360
        _ => DeviceClass::Other, // every other keyboard is Other, or the id no longer resolves
    }
}

// cache: HashMap<SourceId, DeviceClass>, filled on first sight.
fn class_of(cache: &mut HashMap<SourceId, DeviceClass>, device: Option<SourceId>) -> DeviceClass {
    match device {
        None => DeviceClass::Injected,
        Some(id) => *cache.entry(id).or_insert_with(|| classify(id)),
    }
}
```

## Cost, stated plainly

- No remapping inside secure input (password fields): the CGEventTap is bypassed there, so figaro does not see or remap those keys. This is the one thing the HID route would have bought and this does not; it is out of scope here (see `hid-backend.md`, the deferred secure-input upgrade).
- The two device symbols are private and undocumented. They are stable across years and ship in notarized apps, but a future macOS could change them. Every call guards for `NULL`/`0` and degrades to `device: None` rather than failing, so a break costs device-awareness, not the remapper.

## Tests

- `prop_bool`/`prop_u32`/`prop_string` read the right CF types, and walk to a parent when the entry itself lacks the key (the built-in keyboard's `Built-In` lives up the plane). This is where the spike's `Built-In` = `?` gets fixed, so it is worth a real test against a known service.
- `source_of` returns `None` for a synthetic event (built with `CGEvent::new_keyboard_event`, no HID origin) and `Some` for the hardware path, which is the manual spike, not a unit test.
- A `KeyEvent` with `device: None` renders without the field under `Debug`, matching the `flags` treatment.

The end-to-end proof is the spike in this session: two physical keyboards resolved by name off the tap, and it is the acceptance test for the leaf crate — run it, type on two keyboards, see two devices.
