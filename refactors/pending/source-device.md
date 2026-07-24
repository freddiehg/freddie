# source-device attribution on the CGEventTap

One half of device-conditioned input. This doc is the freddie half: each physical source is resolved once, categorized once into a consumer-chosen `T`, and every later key from that source carries that `T`. The figaro half is `figaro/refactors/pending/device-conditioned-keymaps.md` (`DeviceClass` as `T`, model events, layer gate, `On`).

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

`SourceId` is the cache key and never appears on the key callback. Resolve yields a `DeviceInfo` (rich, including a product name string) only as input to the consumer's categorize function, once per `SourceId`. What the key callback sees is `T`, not `DeviceInfo`. `KeyEvent` carries neither; `freddie_keys` never learns they exist.

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
/// Built only on first sight of a `SourceId`, as the argument to the consumer's categorize
/// function. Not cached and not handed to `on_key` — only the categorize result `T` is.
/// A product name `String` is fine here: this path runs once per device per process.
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub built_in: bool,
}

/// Registry lookup for a `SourceId`. `None` if nothing matches (device gone since the press).
/// Called only on a cache miss inside `intercept_with_source`, then fed to categorize.
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
        product: prop_string(service, "Product").unwrap_or_default(),
        built_in: prop_bool(service, "Built-In").unwrap_or(false),
    };
    // SAFETY: release the +1 service.
    #[expect(unsafe_code)]
    unsafe {
        IOObjectRelease(service);
    }
    Some(info)
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

`KeyEvent` and `freddie_keys` do not change. `freddie_keyboard` exposes two entry points over one internal tap. Only code that holds the `CGEvent` can read the source, so resolve and categorize live here.

Per key, the hot path is: `source_of` → cache lookup by `SourceId` → hand `T` to `on_key`. On a miss: `resolve` (registry walk, may allocate a product `String`) → `categorize(Option<DeviceInfo>)` → store `T` → hand `T` to `on_key`. Categorize may be expensive; it runs once per `SourceId` (and once for the synthetic/no-source case).

```rust
// freddie_keyboard, macOS. mercury uses intercept; figaro uses intercept_with_source.

pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

/// Same tap. `categorize` turns a resolved device (or `None` for synthetic / unresolvable)
/// into a consumer value `T`, once per distinct source. `on_key` sees only `T`.
pub fn intercept_with_source<T, C, F>(
    mut categorize: C,
    on_key: F,
) -> Result<(Interceptor, Emitter), CaptureError>
where
    T: Clone + Send + 'static,
    C: FnMut(Option<DeviceInfo>) -> T + Send + 'static,
    F: Fn((KeyEvent, T)) -> Option<KeyEvent> + Send + 'static;
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

Shared `run_tap` owns the thread and the tap. `intercept` ignores the event. `intercept_with_source` owns the source cache and the categorize function:

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    run_tap(move |input, _event| on_key(input))
}

pub fn intercept_with_source<T, C, F>(
    mut categorize: C,
    on_key: F,
) -> Result<(Interceptor, Emitter), CaptureError>
where
    T: Clone + Send + 'static,
    C: FnMut(Option<DeviceInfo>) -> T + Send + 'static,
    F: Fn((KeyEvent, T)) -> Option<KeyEvent> + Send + 'static,
{
    // Cached categorize results only. DeviceInfo is not retained.
    let mut by_source: HashMap<SourceId, T> = HashMap::new();
    // Synthetic / no HID origin: one categorize(None), reused for every such key.
    let mut no_source: Option<T> = None;

    run_tap(move |input, event| {
        let class = match source_of(event) {
            None => no_source
                .get_or_insert_with(|| categorize(None))
                .clone(),
            Some(id) => by_source
                .entry(id)
                .or_insert_with(|| categorize(resolve(id)))
                .clone(),
        };
        on_key((input, class))
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
}
```

`categorize` arguments:

- `None` — no `SourceId` (synthetic / injected from another app), or `resolve` failed (id gone). Figaro maps both to its policy (e.g. `Injected` vs `Other` only if it can tell them apart; with a single `Option` they share `None`, so figaro treats `None` as non-hardware / unusable for desktop layers — typically `Injected` or `Other` by choice; prefer treating `None` as "no device class worth remapping," i.e. figaro's `Injected`).
- `Some(DeviceInfo)` — full resolve, including `product: String`. Figaro may match vendor/product ids and ignore the name, or log the name; the string never enters the model.

If synthetic and failed-resolve must be distinct later, change the categorize argument to an enum (`Synthetic` / `Gone` / `Device(DeviceInfo)`). One `Option` is enough for figaro today.

The cache is a plain `HashMap` local to the tap-thread closure. It is not `Arc`/`Mutex` and is not shared across threads. Only `T` is stored; `DeviceInfo` is dropped after categorize returns.

`intercept` never calls `source_of`, `resolve`, or categorize.

### Re-exports and deps

```rust
// crates/freddie_keyboard/src/lib.rs
pub use freddie_keys::{Key, KeyEvent, PressType};
#[cfg(target_os = "macos")]
pub use freddie_hid_device::DeviceInfo;
// SourceId / source_of / resolve stay in freddie_hid_device for demos/tests.

// sys/mod.rs
pub use macos::{Emitter, Interceptor, intercept, intercept_with_source};
```

```toml
# crates/freddie_keyboard/Cargo.toml — add under target.'cfg(target_os = "macos")'.dependencies
freddie_hid_device = { path = "../freddie_hid_device", version = "0.0.1" }
```

No feature gate. mercury depends on `freddie_keyboard` as today and keeps calling `intercept`.

## Hand-off to figaro

This doc ends at `(KeyEvent, T)` on the callback, with figaro supplying `T = DeviceClass` via categorize. Model events and keymap gates are `device-conditioned-keymaps.md`.

```rust
// figaro (summary)
intercept_with_source(
    |info| match info {
        None => DeviceClass::Injected,
        Some(d) if d.built_in => DeviceClass::Laptop,
        Some(d) if d.vendor_id == 0x29ea && d.product_id == 0x0360 => DeviceClass::Desktop,
        Some(_) => DeviceClass::Other,
    },
    |(key, class)| {
        let _ = raw_tx.send((key, class));
        None
    },
);
```

## Cost, stated plainly

- No remapping inside secure input (password fields): the CGEventTap is bypassed there. That is the one thing the HID route would still buy; out of scope here (`hid-backend.md`).
- The two device symbols are private. Missing HID origin degrades to `categorize(None)` rather than failing the remapper.
- First key from a newly seen device does a registry walk and one categorize on the tap thread. That is once per attachment per process; later keys are a map lookup and a `T::clone`. Prefer a cheap `T` (`Copy` or a small enum).

## Tests

- `prop_bool` / `prop_u32` / `prop_string` read the right CF types and walk to a parent when the entry itself lacks the key (built-in keyboard's `Built-In` lives up the plane). Real test against a known attached service when running on macOS with a keyboard.
- `source_of` returns `None` for a synthetic event (`CGEvent::new_keyboard_event`, no HID origin). Hardware path: two keyboards yield two `SourceId`s and two categorize calls.
- Cache: second key with the same `SourceId` does not call `resolve` or `categorize` again (demo that counts categorize invocations).

## Order of changes

Each step is independently shippable.

1. `freddie_hid_device` leaf: `SourceId`, `source_of`, `resolve`, `DeviceInfo`, `prop_*` as above. Workspace member. Demo: listen tap, print `resolve(source_of(e))` per key. No `freddie_keyboard` change.
2. `freddie_keyboard`: extract `run_tap`, add generic `intercept_with_source` with categorize + `HashMap<SourceId, T>`. `intercept` stays the thin wrapper. mercury still calls `intercept`.
3. Stop. Figaro work is the other doc (`device-conditioned-keymaps.md`).
