# source-device attribution on the CGEventTap

One half of device-conditioned input. This doc is the freddie half: each key gets an `Option<DeviceInfo>` off the CGEventTap. The figaro half is `figaro/refactors/pending/device-conditioned-keymaps.md` (classify into `DeviceClass`, pair events, layer gate, `On`).

No seize, no virtual device, no root, no Karabiner: the source device is read directly off each `CGEvent`. figaro calls `intercept_with_source`; mercury keeps plain `intercept` and never sees a device.

Verified on real hardware: with no seizing remapper running, a listen tap resolves `Kinesis Adv360` and `Apple Internal Keyboard / Trackpad` as distinct devices straight from the events. The technique ships in LinearMouse (notarized, no entitlement, hardened runtime).

## The mechanism

Each `CGEvent` from a hardware key wraps the kernel `IOHIDEvent` it came from, and that event names its source device. Two private symbols reach it, plus public IOKit to resolve the id to a device:

```c
IOHIDEventRef CGEventCopyIOHIDEvent(CGEventRef);   // CoreGraphics, private; NULL for synthetic events
uint64_t      IOHIDEventGetSenderID(IOHIDEventRef); // IOKit SPI; the registry entry id of the source HID service
```

`senderID` is an IOKit registry entry id. `IORegistryEntryIDMatching(senderID)` → `IOServiceGetMatchingService` → walk parents to the `IOHIDDevice` → read `VendorID`, `ProductID`, `Product`, `Built-In`.

Injected and software-synthesized events (untagged posts from other apps) have no HID backing: `CGEventCopyIOHIDEvent` returns `NULL` or the sender id is `0`. A real-hardware key always carries a source; a synthetic one never does. Own emits are tagged by the interceptor and never reach the callback, so the consumer does not classify them.

Two keyboards at once needs no handling: each `CGEvent` independently carries its own source, so simultaneous keys on two devices are attributed separately with nothing to correlate.

## The one requirement

Nothing may be seizing the keyboard upstream. A seizing remapper (Karabiner-Elements' grabber) reads the physical device and re-emits through its own virtual keyboard, so by the time a session tap sees a key, its source is that virtual device, not the real one. With Karabiner running, every key resolves to "Karabiner DriverKit VirtualHIDKeyboard"; with it quit, the real devices appear. figaro is the sole grabber: it replaces Karabiner-Elements.

## The leaf crate

The two private symbols are not in the `core-graphics`/`io-kit-sys` safe wrappers, so the FFI lives in a leaf crate, `freddie_hid_device`, with its own lint table (`unsafe_code = "deny"`, same shape as `freddie_windows` / planned `freddie_keyboard_win_sys`). Everything above it stays under workspace `forbid(unsafe_code)`.

`SourceId` is the cache key for resolve and never appears in the consumer callback. Consumers of `intercept_with_source` get a resolved `DeviceInfo`. `KeyEvent` carries neither; `freddie_keys` never learns they exist.

```rust
// crates/freddie_hid_device/Cargo.toml
[package]
name = "freddie_hid_device"
description = "CGEvent source-device identity and IOKit resolve for freddie."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
core-foundation = { version = "0.10", features = ["link"] }
core-graphics = { version = "0.25", features = ["link"] }
io-kit-sys = "0.4"

[lints.rust]
unsafe_code = "deny"

[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery = { level = "deny", priority = -1 }
cargo = { level = "deny", priority = -1 }
multiple_crate_versions = "allow"
cargo_common_metadata = "allow"
empty_structs_with_brackets = "deny"
```

Workspace `members` gains `"crates/freddie_hid_device"`.

```rust
// crates/freddie_hid_device/src/lib.rs

use std::ffi::c_void;
use std::os::raw::c_char;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::{CFBoolean, CFBooleanGetTypeID, CFBooleanGetValue};
use core_foundation::number::{CFNumber, CFNumberGetTypeID, CFNumberRef};
use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
use core_graphics::event::CGEvent;
use io_kit_sys::ret::KERN_SUCCESS;
use io_kit_sys::types::{io_object_t, io_registry_entry_t, IO_OBJECT_NULL};
use io_kit_sys::{
    kIOMasterPortDefault, IOObjectRelease, IORegistryEntryCreateCFProperty,
    IORegistryEntryGetParentEntry, IORegistryEntryIDMatching, IOServiceGetMatchingService,
};

/// Registry entry id of the originating HID service. Stable while the device stays attached;
/// a replug yields a new one. Cache key inside `freddie_keyboard`; not part of the consumer
/// callback.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SourceId(pub u64);

type CGEventRef = *const c_void;
type IOHIDEventRef = *mut c_void;

#[expect(unsafe_code)]
unsafe extern "C" {
    fn CGEventCopyIOHIDEvent(event: CGEventRef) -> IOHIDEventRef;
    fn IOHIDEventGetSenderID(event: IOHIDEventRef) -> u64;
}

/// Source HID service of a `CGEvent`, or `None` for an injected/synthetic event. Two FFI calls
/// and a release; no registry walk.
pub fn source_of(event: &CGEvent) -> Option<SourceId> {
    // SAFETY: private CoreGraphics symbol; returns +1 IOHIDEvent or null when no HID origin.
    #[expect(unsafe_code)]
    let hid = unsafe { CGEventCopyIOHIDEvent(event.as_ptr().cast()) };
    if hid.is_null() {
        return None;
    }
    // SAFETY: `hid` is live +1; read sender, then drop our reference.
    #[expect(unsafe_code)]
    let sender = unsafe { IOHIDEventGetSenderID(hid) };
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(hid.cast());
    }
    (sender != 0).then_some(SourceId(sender))
}

/// Resolved identity of a source. Fields come off the service the id names, or the nearest
/// ancestor that carries them (some live on the parent `IOHIDDevice`).
///
/// `product` is interned (`&'static str`): `Copy`, free to hand across the channel, and safe to
/// keep on a cached `DeviceInfo` for the process lifetime. See `intern_product`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: &'static str,
    pub built_in: bool,
}

/// Registry lookup for a `SourceId`. `None` if nothing matches (device gone since the press).
/// `intercept_with_source` caches successful results by `SourceId`.
pub fn resolve(id: SourceId) -> Option<DeviceInfo> {
    // SAFETY: IORegistryEntryIDMatching returns +1 dict; IOServiceGetMatchingService consumes
    // it and returns +1 service, or IO_OBJECT_NULL when nothing matches.
    #[expect(unsafe_code)]
    let service = unsafe {
        IOServiceGetMatchingService(kIOMasterPortDefault, IORegistryEntryIDMatching(id.0))
    };
    if service == IO_OBJECT_NULL {
        return None;
    }
    let info = DeviceInfo {
        vendor_id: prop_u32(service, "VendorID").unwrap_or(0) as u16,
        product_id: prop_u32(service, "ProductID").unwrap_or(0) as u16,
        product: intern_product(prop_string(service, "Product").unwrap_or_default()),
        built_in: prop_bool(service, "Built-In").unwrap_or(false),
    };
    // SAFETY: release the +1 service.
    #[expect(unsafe_code)]
    unsafe {
        IOObjectRelease(service);
    }
    Some(info)
}

/// Process-lifetime intern of a product name. Product strings are a closed set per machine
/// (a handful of keyboards); leaking one copy per distinct name (or per resolve without a
/// content-keyed pool) is the cost of `&'static str` without a shared mutable intern table.
/// Empty input is `""` (no leak). Not `Arc`/`Mutex`/`static` mut: a pure leak of an owned
/// `String` that will never be freed.
fn intern_product(s: String) -> &'static str {
    if s.is_empty() {
        return "";
    }
    Box::leak(s.into_boxed_str())
}

/// Walk `entry` and its parents on the service plane until `key` is present with the expected
/// CF type. `Built-In` is CFBoolean; `VendorID`/`ProductID` are CFNumber; `Product` is CFString.
fn prop_bool(entry: io_registry_entry_t, key: &str) -> Option<bool> {
    with_prop(entry, key, CFBooleanGetTypeID(), |raw| {
        // SAFETY: type id matched CFBoolean.
        #[expect(unsafe_code)]
        Some(unsafe { CFBooleanGetValue(raw.cast()) })
    })
}

fn prop_u32(entry: io_registry_entry_t, key: &str) -> Option<u32> {
    with_prop(entry, key, CFNumberGetTypeID(), |raw| {
        let n = unsafe {
            // SAFETY: type id matched CFNumber; wrap without taking ownership of the Get ref.
            CFNumber::wrap_under_get_rule(raw as CFNumberRef)
        };
        n.to_i64().and_then(|v| u32::try_from(v).ok())
    })
}

fn prop_string(entry: io_registry_entry_t, key: &str) -> Option<String> {
    with_prop(entry, key, CFStringGetTypeID(), |raw| {
        let s = unsafe {
            // SAFETY: type id matched CFString; wrap without taking ownership of the Get ref.
            CFString::wrap_under_get_rule(raw as CFStringRef)
        };
        Some(s.to_string())
    })
}

fn with_prop<T>(
    entry: io_registry_entry_t,
    key: &str,
    want_type: core_foundation::base::CFTypeID,
    read: impl Fn(CFTypeRef) -> Option<T>,
) -> Option<T> {
    let cf_key = CFString::new(key);
    let mut current: io_registry_entry_t = entry;
    // First entry is borrowed from the caller; each parent we create is +1 and must be released.
    let mut owned: Option<io_object_t> = None;
    loop {
        // SAFETY: CreateCFProperty returns +1 or null; key is a live CFString.
        #[expect(unsafe_code)]
        let prop = unsafe {
            IORegistryEntryCreateCFProperty(
                current,
                cf_key.as_concrete_TypeRef(),
                std::ptr::null_mut(),
                0,
            )
        };
        if !prop.is_null() {
            // SAFETY: prop is +1 CFType.
            #[expect(unsafe_code)]
            let type_id = unsafe { core_foundation::base::CFGetTypeID(prop) };
            let out = if type_id == want_type {
                read(prop)
            } else {
                None
            };
            #[expect(unsafe_code)]
            unsafe {
                CFRelease(prop);
            }
            if let Some(owned) = owned {
                #[expect(unsafe_code)]
                unsafe {
                    IOObjectRelease(owned);
                }
            }
            return out;
        }
        let mut parent: io_registry_entry_t = IO_OBJECT_NULL;
        // SAFETY: service plane parent walk.
        #[expect(unsafe_code)]
        let kr = unsafe {
            IORegistryEntryGetParentEntry(
                current,
                c"IOService".as_ptr().cast::<c_char>(),
                &mut parent,
            )
        };
        if let Some(owned) = owned {
            #[expect(unsafe_code)]
            unsafe {
                IOObjectRelease(owned);
            }
        }
        if kr != KERN_SUCCESS || parent == IO_OBJECT_NULL {
            return None;
        }
        current = parent;
        owned = Some(parent);
    }
}
```

`CGEventCopyIOHIDEvent` and `IOHIDEventGetSenderID` are the only private symbols; everything `resolve` calls is public IOKit. The leaf stamps an id onto an event and turns an id into a `DeviceInfo`. Which device is which is the consumer's policy.

`kIOMasterPortDefault` is the symbol `io-kit-sys` exposes today; if a newer SDK renames it to `kIOMainPortDefault`, switch the import and keep the call the same.

## The device-aware entry point

`KeyEvent` and `freddie_keys` do not change. The device rides alongside the key: `freddie_keyboard` exposes two entry points over one internal tap. Only code that holds the `CGEvent` can read the source, so the resolve lives here.

```rust
// freddie_keyboard, macOS. mercury uses intercept; figaro uses intercept_with_source.

pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

// One argument: key paired with resolved device (`None` = synthetic / no HID origin).
pub fn intercept_with_source(
    on_key: impl Fn((KeyEvent, Option<DeviceInfo>)) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;
```

### Before (`sys/macos.rs`)

One public entry point; the tap body is inline inside `intercept`.

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    // ... spawn thread, CGEventTap::with_enabled, build KeyEvent, on_key(input), decide ...
}
```

### After

Shared `run_tap` owns the thread and the tap. The two entry points only differ in how they build the callback argument:

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    run_tap(move |input, _event| on_key(input))
}

pub fn intercept_with_source(
    on_key: impl Fn((KeyEvent, Option<DeviceInfo>)) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    let mut cache: HashMap<SourceId, DeviceInfo> = HashMap::new();
    run_tap(move |input, event| {
        let device = source_of(event).and_then(|id| {
            if let Some(&cached) = cache.get(&id) {
                return Some(cached); // Copy
            }
            let info = resolve(id)?; // interns product once per SourceId (cache miss)
            cache.insert(id, info);
            Some(info)
        });
        on_key((input, device))
    })
}

/// Shared tap install. `on_key` already decided pass/remap/drop via its return.
/// `event` is the live `CGEvent` for the key; only `intercept_with_source` reads it.
fn run_tap(
    mut on_key: impl FnMut(KeyEvent, &CGEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    // same install as today's intercept: tag, thread, CGEventTap::with_enabled,
    // press_of / from_code / flags, then:
    //   match decide(&input, on_key(input.clone(), event)) { Pass / Drop / Remap }
    // cache and SourceId never leave this module except through DeviceInfo on the pair.
}
```

The cache is a plain `HashMap` local to the tap-thread closure. It is not `Arc`/`Mutex` and is not shared across threads: only the tap thread mutates it. `DeviceInfo` is `Copy` (`product: &'static str`), so a cache hit is a load, not a `String` clone, and the value is free to send to the worker.

`intercept` never calls `source_of` or `resolve`. `intercept_with_source` does one registry walk (and at most one product intern) on first sight of a `SourceId` and a map hit thereafter.

### Re-exports and deps

```rust
// crates/freddie_keyboard/src/lib.rs
pub use freddie_keys::{Key, KeyEvent, PressType};
#[cfg(target_os = "macos")]
pub use freddie_hid_device::DeviceInfo;
// SourceId / source_of / resolve stay in freddie_hid_device for demos/tests; not re-exported
// from freddie_keyboard unless a consumer needs them.

// sys/mod.rs
pub use macos::{Emitter, Interceptor, intercept, intercept_with_source};
```

```toml
# crates/freddie_keyboard/Cargo.toml — add under target.'cfg(target_os = "macos")'.dependencies
freddie_hid_device = { path = "../freddie_hid_device", version = "0.0.1" }
```

No feature gate. mercury depends on `freddie_keyboard` as today and keeps calling `intercept`.

## Hand-off to figaro

This doc ends at `(KeyEvent, Option<DeviceInfo>)` on the callback. Classification, model pairs, and keymap gates are `device-conditioned-keymaps.md`.

## Cost, stated plainly

- No remapping inside secure input (password fields): the CGEventTap is bypassed there. That is the one thing the HID route would still buy; out of scope here (`hid-backend.md`).
- The two device symbols are private. Every call guards for `NULL`/`0` and degrades to `None` (no `DeviceInfo`) rather than failing, so a break costs device-awareness, not the remapper.
- First key from a newly seen device does a registry walk on the tap thread. That is once per attachment per process; later keys are a map lookup.

## Tests

- `prop_bool` / `prop_u32` / `prop_string` read the right CF types and walk to a parent when the entry itself lacks the key (built-in keyboard's `Built-In` lives up the plane). Real test against a known attached service when running on macOS with a keyboard.
- `source_of` returns `None` for a synthetic event (`CGEvent::new_keyboard_event`, no HID origin). Hardware `Some` is a manual check: type on two keyboards, see two `DeviceInfo` product names.
- Resolve cache: second key with the same `SourceId` does not call `resolve` again (exercise via a small demo binary that logs, or a test seam if one is introduced).

## Order of changes

Each step is independently shippable.

1. `freddie_hid_device` leaf: `SourceId`, `source_of`, `resolve`, `DeviceInfo`, `prop_*` as above. Workspace member. Demo binary (or `examples/`) that installs a listen-only tap, prints `resolve(source_of(e))` per key, and is the end-to-end proof of the leaf. No `freddie_keyboard` change.
2. `freddie_keyboard`: extract `run_tap`, add `intercept_with_source` with the tap-thread cache, re-export `DeviceInfo`. `intercept` becomes the thin wrapper that ignores the event. mercury binary still compiles and behaves as today (still calls `intercept`).
3. Stop. Figaro work is the other doc (`device-conditioned-keymaps.md`), after richer keys.
