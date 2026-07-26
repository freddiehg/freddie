//! `CGEvent` source-device identity and `IOKit` resolve.
//!
//! Private SPI: `CGEventCopyIOHIDEvent` / `IOHIDEventGetSenderID`. Public `IOKit` for
//! registry property walks and the `HIDSystem` modifier-lock latch. macOS only.

#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};

use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeID, CFTypeRef, TCFType};
use core_foundation::boolean::{CFBooleanGetTypeID, kCFBooleanTrue};
use core_foundation::number::{CFNumber, CFNumberGetTypeID};
use core_foundation::string::{CFString, CFStringGetTypeID};
use core_graphics::event::CGEvent;
use foreign_types_shared::ForeignType;
use io_kit_sys::types::{IO_OBJECT_NULL, io_connect_t, io_object_t, io_registry_entry_t};
use io_kit_sys::{
    IOObjectRelease, IORegistryEntryCreateCFProperty, IORegistryEntryGetParentEntry,
    IORegistryEntryIDMatching, IOServiceClose, IOServiceGetMatchingService, IOServiceMatching,
    IOServiceOpen, kIOMasterPortDefault,
};
use mach2::kern_return::{KERN_SUCCESS, kern_return_t};
use mach2::traps::mach_task_self;

/// Registry entry id of the originating HID service. Stable while the device stays attached;
/// a replug yields a new one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SourceId(pub u64);

type CGEventRef = *const c_void;
type IOHIDEventRef = *mut c_void;

/// `kIOHIDCapsLockState` — selector for [`IOHIDSetModifierLockState`].
const IOHID_CAPS_LOCK_STATE: c_int = 0x0000_0001;

/// `kIOHIDParamConnectType` — user client type for `HIDSystem` parameter access.
const IOHID_PARAM_CONNECT_TYPE: u32 = 1;

#[expect(unsafe_code)]
unsafe extern "C" {
    fn CGEventCopyIOHIDEvent(event: CGEventRef) -> IOHIDEventRef;
    fn IOHIDEventGetSenderID(event: IOHIDEventRef) -> u64;

    /// Force the session Caps Lock (and LED) on or off. A `CGEventPost` of `CapsLock` does not
    /// drive this latch on modern macOS; this is the public `HIDSystem` API that does.
    fn IOHIDSetModifierLockState(
        handle: io_connect_t,
        selector: c_int,
        state: bool,
    ) -> kern_return_t;
}

/// Why [`set_caps_lock`] failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapsLockError {
    /// No `IOHIDSystem` service in the registry.
    NoHidSystem,
    /// `IOServiceOpen` for the parameter user client failed.
    OpenFailed,
    /// `IOHIDSetModifierLockState` returned a non-success kern return.
    SetFailed,
}

/// Force the session Caps Lock latch (and keyboard LED) on or off.
///
/// Physical `CapsLock` still toggles this latch even when a `CGEventTap` drops the event.
/// Posting a synthetic `CapsLock` via CoreGraphics does not reverse it; this call does.
///
/// # Errors
///
/// Returns [`CapsLockError`] when the `HIDSystem` user client cannot be opened or the set fails.
pub fn set_caps_lock(on: bool) -> Result<(), CapsLockError> {
    // SAFETY: IOServiceMatching returns +1 dict consumed by IOServiceGetMatchingService;
    // service is +1 or null.
    #[expect(unsafe_code)]
    let service = unsafe {
        IOServiceGetMatchingService(
            kIOMasterPortDefault,
            IOServiceMatching(c"IOHIDSystem".as_ptr().cast::<c_char>()),
        )
    };
    if service == IO_OBJECT_NULL {
        return Err(CapsLockError::NoHidSystem);
    }
    let mut connect: io_connect_t = IO_OBJECT_NULL;
    // SAFETY: open parameter user client on this task; connect out-param written on success.
    #[expect(unsafe_code)]
    let open_kr = unsafe {
        IOServiceOpen(
            service,
            mach_task_self(),
            IOHID_PARAM_CONNECT_TYPE,
            &raw mut connect,
        )
    };
    // SAFETY: release the +1 service either way.
    #[expect(unsafe_code)]
    unsafe {
        let _ = IOObjectRelease(service);
    }
    if open_kr != KERN_SUCCESS || connect == IO_OBJECT_NULL {
        return Err(CapsLockError::OpenFailed);
    }
    // SAFETY: connect is a live param user client from IOServiceOpen above.
    #[expect(unsafe_code)]
    let set_kr = unsafe { IOHIDSetModifierLockState(connect, IOHID_CAPS_LOCK_STATE, on) };
    // SAFETY: close the user client we opened.
    #[expect(unsafe_code)]
    unsafe {
        let _ = IOServiceClose(connect);
    }
    if set_kr == KERN_SUCCESS {
        Ok(())
    } else {
        Err(CapsLockError::SetFailed)
    }
}

/// Source HID service of a `CGEvent`, or `None` for an injected/synthetic event.
#[must_use]
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

/// Resolved identity of a source for the categorize path (once per `SourceId`).
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub built_in: bool,
}

/// Why [`resolve`] failed for a known [`SourceId`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ResolveFailure {
    /// `IOServiceGetMatchingService` returned no service.
    NoMatchingService,
}

/// Registry lookup for a [`SourceId`].
///
/// # Errors
///
/// Returns `(id, reason)` when the id matches no live service.
pub fn resolve(id: SourceId) -> Result<DeviceInfo, (SourceId, ResolveFailure)> {
    // SAFETY: IORegistryEntryIDMatching returns +1 dict; IOServiceGetMatchingService consumes
    // it and returns +1 service, or IO_OBJECT_NULL when nothing matches.
    #[expect(unsafe_code)]
    let service = unsafe {
        IOServiceGetMatchingService(kIOMasterPortDefault, IORegistryEntryIDMatching(id.0))
    };
    if service == IO_OBJECT_NULL {
        return Err((id, ResolveFailure::NoMatchingService));
    }
    let info = DeviceInfo {
        vendor_id: prop_u32(service, "VendorID")
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(0),
        product_id: prop_u32(service, "ProductID")
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(0),
        product: prop_string(service, "Product").unwrap_or_default(),
        built_in: prop_bool(service, "Built-In").unwrap_or(false),
    };
    // SAFETY: release the +1 service.
    #[expect(unsafe_code)]
    unsafe {
        let _ = IOObjectRelease(service);
    }
    Ok(info)
}

fn prop_bool(entry: io_registry_entry_t, key: &str) -> Option<bool> {
    // SAFETY: CFBooleanGetTypeID is a pure type-id query.
    #[expect(unsafe_code)]
    let ty = unsafe { CFBooleanGetTypeID() };
    with_prop(entry, key, ty, |raw| {
        // SAFETY: type id matched CFBoolean; true is the global kCFBooleanTrue singleton.
        #[expect(unsafe_code)]
        let is_true = unsafe { raw.cast() == kCFBooleanTrue };
        Some(is_true)
    })
}

fn prop_u32(entry: io_registry_entry_t, key: &str) -> Option<u32> {
    // SAFETY: pure type-id query.
    #[expect(unsafe_code)]
    let ty = unsafe { CFNumberGetTypeID() };
    with_prop(entry, key, ty, |raw| {
        // SAFETY: type id matched CFNumber; wrap without taking ownership of the Get ref.
        #[expect(unsafe_code)]
        let n = unsafe { CFNumber::wrap_under_get_rule(raw.cast()) };
        n.to_i64().and_then(|v| u32::try_from(v).ok())
    })
}

fn prop_string(entry: io_registry_entry_t, key: &str) -> Option<String> {
    // SAFETY: pure type-id query.
    #[expect(unsafe_code)]
    let ty = unsafe { CFStringGetTypeID() };
    with_prop(entry, key, ty, |raw| {
        // SAFETY: type id matched CFString; wrap without taking ownership of the Get ref.
        #[expect(unsafe_code)]
        let s = unsafe { CFString::wrap_under_get_rule(raw.cast()) };
        Some(s.to_string())
    })
}

fn with_prop<T>(
    entry: io_registry_entry_t,
    key: &str,
    want_type: CFTypeID,
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
                core_foundation::base::kCFAllocatorDefault,
                0,
            )
        };
        if !prop.is_null() {
            // SAFETY: prop is +1 CFType.
            #[expect(unsafe_code)]
            let type_id = unsafe { CFGetTypeID(prop) };
            let out = if type_id == want_type {
                read(prop)
            } else {
                None
            };
            #[expect(unsafe_code)]
            unsafe {
                CFRelease(prop);
            }
            if let Some(prev) = owned {
                #[expect(unsafe_code)]
                unsafe {
                    let _ = IOObjectRelease(prev);
                }
            }
            return out;
        }
        let mut parent: io_registry_entry_t = IO_OBJECT_NULL;
        // SAFETY: service plane parent walk; parent is +1 when successful.
        #[expect(unsafe_code)]
        let kr = unsafe {
            IORegistryEntryGetParentEntry(
                current,
                c"IOService".as_ptr().cast::<c_char>(),
                &raw mut parent,
            )
        };
        if let Some(prev) = owned {
            #[expect(unsafe_code)]
            unsafe {
                let _ = IOObjectRelease(prev);
            }
        }
        if kr != KERN_SUCCESS || parent == IO_OBJECT_NULL {
            return None;
        }
        current = parent;
        owned = Some(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_caps_lock_off_reaches_hidsystem() {
        // Does not assert prior latch state; only that the user client opens and accepts off.
        set_caps_lock(false).expect("IOHIDSetModifierLockState(false)");
    }
}
