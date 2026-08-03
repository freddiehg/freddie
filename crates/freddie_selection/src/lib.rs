//! Watching every app's selected text, and reading it.
//!
//! The shape `freddie_windows` has after its watcher fixes: a watcher that reports facts, and
//! a read the consumer's effect performers call.
//!
//! - [`watch`] is the source. One `AXObserver` per observable app (on
//!   `freddie_ax_observer`'s scaffolding) reports that an app's selection changed, with no
//!   value: the consumer's model requests it as a read effect when it hears the fact. The
//!   install pass reports a `Changed` for every running app, queued as events that dispatch
//!   after the consumer's model exists, so the seed path and the steady-state path are the
//!   same code and the consumer's map starts empty and honest.
//! - [`current_selection`] is the read, called by the consumer's effect performers and nobody
//!   else — never by dispatch, never by this watcher.
//!
//! Chrome and Electron web content answer [`Selection::Unsupported`]: their accessibility
//! trees are off until an assistive client announces itself, and nothing here sets the
//! activation flags.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::ffi::c_void;
use std::rc::Rc;

use accessibility_sys::{
    AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef,
    kAXFocusedUIElementAttribute, kAXFocusedUIElementChangedNotification, kAXSelectedTextAttribute,
    kAXSelectedTextChangedNotification,
};
use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use freddie_ax_observer::{AppSeen, AppWatch, add_notification, watch_apps};

pub use freddie_ax_observer::Pid;
pub use freddie_selection_types::{Selection, SelectionChange};

/// What a notification callback needs: which app it is about, and where to report.
///
/// The callback reports the fact and returns; there is nothing else for it to reach.
struct Registration {
    pid: Pid,
    on_change: Rc<dyn Fn(SelectionChange)>,
}

/// The one `AXObserver` callback: whichever of the two registered notifications fired, the
/// fact is the same — this app's selection is dead.
///
/// Runs on the main thread, since that is the run loop the sources were added to.
#[expect(unsafe_code)]
unsafe extern "C" fn on_notification(
    _observer: accessibility_sys::AXObserverRef,
    _element: AXUIElementRef,
    _notification: CFStringRef,
    refcon: *mut c_void,
) {
    // SAFETY: `refcon` is the `Box<Registration>` this app's observer still owns. The
    // observer's source is removed before the box is dropped, so no notification can arrive
    // after the pointer goes stale.
    let registration = unsafe { &*refcon.cast::<Registration>() };
    (registration.on_change)(SelectionChange::Changed(registration.pid));
}

/// Holds the per-app observers that make selections report. While one of these is alive,
/// changes reach the `on_change` it was built with; dropping it stops them.
pub struct SelectionWatch {
    _apps: AppWatch<Registration>,
}

/// Watch every app's selection. Installed on the main thread, like `freddie_windows::watch`.
///
/// `on_change` runs on the main thread, serialized with every other main-thread callback,
/// so it must hand its work elsewhere and return. Sending on a channel is the intended
/// body.
pub fn watch(on_change: impl Fn(SelectionChange) + 'static) -> SelectionWatch {
    let on_change: Rc<dyn Fn(SelectionChange)> = Rc::new(on_change);
    let make_change = Rc::clone(&on_change);
    let install_change = Rc::clone(&on_change);
    let apps = watch_apps(
        on_notification,
        move |seen| Registration {
            pid: seen.pid,
            on_change: Rc::clone(&make_change),
        },
        move |seen: &AppSeen, refcon| {
            for notification in [
                kAXSelectedTextChangedNotification,
                kAXFocusedUIElementChangedNotification,
            ] {
                // SAFETY: `seen` is live for this callback, and `refcon` is the boxed
                // registration the app's observer owns, freed only after the observer
                // stops delivering.
                #[expect(unsafe_code)]
                unsafe {
                    add_notification(seen.observer, seen.app_element, notification, refcon);
                }
            }
            // During the install pass this is the seed: whatever the consumer knew for this
            // pid — nothing, at boot — is dead, and its model requests the read.
            install_change(SelectionChange::Changed(seen.pid));
        },
        move |pid| on_change(SelectionChange::AppGone(pid)),
    );
    SelectionWatch { _apps: apps }
}

/// What `pid`'s focused element answers right now. One synchronous round-trip into the app;
/// callable from any thread, and the app element it creates is its own and per call.
#[must_use]
pub fn current_selection(pid: Pid) -> Selection {
    // SAFETY: `pid` names a process; the element is +1, released with the `Owned`.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid.0) };
    let Some(app) = Owned::new(app.cast()) else {
        return Selection::Unsupported;
    };
    let Some(focused) = copy_attribute(element(&app), kAXFocusedUIElementAttribute) else {
        return Selection::Unsupported;
    };
    let Some(value) = copy_attribute(element(&focused), kAXSelectedTextAttribute) else {
        return Selection::Unsupported;
    };
    match string_of(&value) {
        Some(text) if text.is_empty() => Selection::Empty,
        Some(text) => Selection::Text(text),
        None => Selection::Unsupported,
    }
}

/// A +1 CoreFoundation reference, released when it drops.
///
/// Deliberately not `Copy` and not `Clone`: two of these naming one reference would
/// release it twice.
struct Owned(CFTypeRef);

impl Owned {
    /// Take ownership of what a `Create` or `Copy` returned, or `None` if it returned
    /// nothing.
    fn new(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: an `Owned` is only built from a +1 reference, and only here is it
        // released, once.
        #[expect(unsafe_code)]
        unsafe {
            CFRelease(self.0);
        }
    }
}

/// The reference as the `AXUIElementRef` the Accessibility calls take. Borrowed, not owned:
/// the release stays with the [`Owned`].
const fn element(owned: &Owned) -> AXUIElementRef {
    owned.0.cast_mut().cast()
}

/// The value of one attribute of `element`, owned.
fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<Owned> {
    let attribute = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is live and `attribute` a live string. On success the
    // out-parameter receives a +1 reference; on failure it is untouched.
    #[expect(unsafe_code)]
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    (status == 0).then(|| Owned::new(value))?
}

/// The value as a `String` when it is the `CFString` the attribute is documented to hold.
///
/// `None`, with a `warn!`, when it is something else: that is the app's Accessibility
/// implementation misbehaving, and a watcher should not die over it.
fn string_of(value: &Owned) -> Option<String> {
    // SAFETY: `value` holds a live CF object; asking its type takes no ownership.
    #[expect(unsafe_code)]
    let is_string = unsafe { CFGetTypeID(value.0) } == CFString::type_id();
    if !is_string {
        tracing::warn!("a selected-text attribute was not a string");
        return None;
    }
    // SAFETY: just checked to be a `CFString`; wrapping under the get rule retains, so the
    // `Owned` still releases exactly what it holds.
    #[expect(unsafe_code)]
    let string = unsafe { CFString::wrap_under_get_rule(value.0.cast()) };
    Some(string.to_string())
}
