//! Watching the windows on screen, and moving them.
//!
//! The shape `freddie_app_nav` has: a source, a sink, and a seed.
//!
//! - [`watch`] is the source. One `AXObserver` per app reports windows opening, moving,
//!   resizing, and closing, plus which one is focused; an `NSWorkspace` observer keeps that
//!   set current as apps launch and quit, and a screen observer reports the monitors. Every
//!   callback runs on the main thread, from its run loop.
//! - [`WindowSink::set_frame`] is the sink. It moves and resizes one window, named by id, to
//!   a rectangle the caller already worked out. It decides nothing: it does not ask what is
//!   frontmost, what is focused, or what the screen looks like.
//! - [`Snapshot`] is the seed, returned by [`watch`] alongside the [`Watcher`]. The observer
//!   reports changes, and at startup nothing has changed yet, so the state a consumer starts
//!   from comes back with the registration that will report every change after it.
//!
//! A window is named by [`WindowId`], its `CGWindowID`, which outlives any one
//! `AXUIElement` for it. The crate keeps the mapping back to an element and nothing outside
//! it ever sees one.
//!
//! Setting a frame goes through the Accessibility API, which is the only way to write one:
//! `CGWindow` can read geometry but not write it. It is immediate, with no animation, and
//! costs single-digit to low tens of milliseconds, so a caller on a latency-sensitive loop
//! should hand it to another thread.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};

use accessibility_sys::{
    AXError, AXIsProcessTrusted, AXObserverAddNotification, AXObserverCreate,
    AXObserverGetRunLoopSource, AXObserverRef, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetAttributeValue, AXValueCreate,
    AXValueGetValue, AXValueType, kAXFocusedWindowAttribute, kAXFocusedWindowChangedNotification,
    kAXPositionAttribute, kAXSizeAttribute, kAXUIElementDestroyedNotification, kAXValueTypeCGPoint,
    kAXValueTypeCGSize, kAXWindowCreatedNotification, kAXWindowMovedNotification,
    kAXWindowResizedNotification, kAXWindowsAttribute, pid_t,
};
use block2::RcBlock;
use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::runloop::{CFRunLoop, CFRunLoopSource, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGSize};
use core_graphics::window::{CGWindowID, kCGNullWindowID};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSApplicationDidChangeScreenParametersNotification,
    NSRunningApplication, NSScreen, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSNotificationName,
};

/// A running app, by process id. `pid_t` is an `i32`, and an `i32` is not a process.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pid(pub pid_t);

/// An app whose windows a user could be looking at, which is the only kind worth observing.
///
/// macOS runs UI services alongside the apps: `CursorUIViewService` draws the text cursor and
/// `Open and Save Panel Service` draws a file dialog, and each of them owns real windows with
/// real ids. Those windows post the same Accessibility notifications an app's windows do, so a
/// watcher that observes every process records one of them as the focused window whenever the
/// user puts a cursor in a text field, and a placement then moves an invisible 64x64 box
/// instead of the window in front of the user.
///
/// Their activation policy is what separates them: `prohibited` means macOS will not let the
/// user bring the app forward at all, so nothing it owns can be what a placement is aimed at.
/// Accessory apps stay in, because a menu bar app has no Dock icon but does have windows, and
/// its settings window is placed like any other.
///
/// Built only by [`Self::of`], so an app that has not been vetted cannot reach
/// [`observe_app`].
#[derive(Clone, Copy, Debug)]
struct ObservableApp(Pid);

impl ObservableApp {
    /// `app` if its windows can be looked at, `None` if it is one of the UI services.
    fn of(app: &NSRunningApplication) -> Option<Self> {
        (app.activationPolicy() != NSApplicationActivationPolicy::Prohibited)
            .then(|| Self(Pid(app.processIdentifier())))
    }
}

/// A window's `CGWindowID`: the identity that outlives any one `AXUIElement` naming it.
///
/// Elements are created per call, so two for the same window are different pointers and
/// the element itself cannot be the key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WindowId(pub CGWindowID);

// SAFETY: `_AXUIElementGetWindow` is exported by HIServices, inside ApplicationServices,
// which this crate already links against for the rest of the Accessibility API. It reads
// the element and writes one `CGWindowID` through the out-parameter.
#[expect(unsafe_code)]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    /// The `CGWindowID` behind an Accessibility window element. Private, and the only
    /// route from an `AXUIElement` to the id the rest of the system names a window by.
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut CGWindowID) -> AXError;
}

/// The window's id, or `None` if it cannot be read. A window without one is placed like
/// any other and is never reported.
fn window_id(window: AXUIElementRef) -> Option<WindowId> {
    let mut id: CGWindowID = kCGNullWindowID;
    // SAFETY: `window` is a live element; the call writes at most one `CGWindowID` into
    // `id` and takes no ownership of either.
    #[expect(unsafe_code)]
    let status = unsafe { _AXUIElementGetWindow(window, &raw mut id) };
    (status == 0 && id != kCGNullWindowID).then_some(WindowId(id))
}

/// A retained `AXUIElement` for one window.
struct Element(Owned);

impl Element {
    /// The element, for the calls that take one. Borrowed, not owned: the release stays
    /// with the [`Owned`] inside.
    ///
    /// Not `as_ref`, which `Arc<Element>` already has from `AsRef` and would shadow this.
    const fn raw(&self) -> AXUIElementRef {
        self.0.0.cast_mut().cast()
    }
}

/// Every window that can be addressed, and the element to address it through.
///
/// A `Mutex` and not an `RwLock`: a window opening and a key being pressed are both rare,
/// so there is nothing for concurrent readers to win. It is held for a lookup and an
/// `Arc::clone`, never across an `AXUIElement` call.
#[derive(Default)]
struct Elements(Mutex<HashMap<WindowId, Arc<Element>>>);

/// The handle a placement is performed through.
///
/// Cheap to clone and unattached to the thread that made it, because the effect loop
/// hands each placement to a thread of its own.
///
/// A [`Weak`], so the watcher is the only thing keeping the table alive and a sink cannot
/// outlive the observation it belongs to.
#[derive(Clone)]
pub struct WindowSink {
    elements: Weak<Elements>,
}

impl WindowSink {
    /// Move and resize one window: `target` names which, and the rectangle it goes to.
    ///
    /// Immediate, with no animation. Costs single-digit to low tens of milliseconds, so a
    /// caller on a latency-sensitive loop should hand this to another thread.
    ///
    /// The frame is the caller's, already worked out. This does not consult the screen,
    /// the frontmost app, or anything else.
    ///
    /// # Errors
    ///
    /// [`WindowError::NotWatching`] if the watcher has been dropped, and
    /// [`WindowError::UnknownWindow`] if nothing with that id is being observed, which is
    /// the case for a window that has closed or that never reported an id.
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        let elements = self.elements.upgrade().ok_or(WindowError::NotWatching)?;
        // Cloned out so the lock is released before the writes: those take tens of
        // milliseconds, and the main thread takes this lock every time a window opens or
        // closes.
        let element = {
            let table = elements.0.lock().map_err(|_| WindowError::UnknownWindow)?;
            Arc::clone(
                table
                    .get(&target.window)
                    .ok_or(WindowError::UnknownWindow)?,
            )
        };
        set_frame(element.raw(), target.frame);
        tracing::debug!(?target, "set a window's frame");
        Ok(())
    }
}

/// A rectangle in Accessibility coordinates: origin top-left, y increasing down.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    /// Whether `(x, y)` lies in this frame. Half-open: the left and top edges are in, the
    /// right and bottom are not, so abutting frames do not both claim a point.
    #[must_use]
    pub const fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// A monitor: its full frame, for locating a window, and its visible frame, the area
/// a placement fills (the full frame minus the menu bar and the dock). Both in
/// Accessibility coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Monitor {
    pub full: Frame,
    pub visible: Frame,
}

/// Placing a window failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowError {
    /// [`watch`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// Nothing is frontmost, or the frontmost app has no focused window.
    NoFocusedWindow,
    /// Nothing with that id is being observed: the window closed, or it never reported an
    /// id to begin with.
    UnknownWindow,
    /// The watcher has been dropped, so nothing is being observed at all.
    NotWatching,
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMainThread => "freddie_windows::watch must run on the main thread",
            Self::NotTrusted => "Accessibility is not granted",
            Self::NoFocusedWindow => "no focused window",
            Self::UnknownWindow => "no such window",
            Self::NotWatching => "not watching windows",
        })
    }
}

impl std::error::Error for WindowError {}

/// Reads every monitor's full and visible frame, in Accessibility coordinates.
///
/// `NSScreen` has a global bottom-left origin and Accessibility a global top-left
/// one, so the y flips around the PRIMARY display's height, not each screen's own.
/// That is what places a monitor above or beside the primary at the right global y.
fn read_monitors(mtm: MainThreadMarker) -> Vec<Monitor> {
    let screens = NSScreen::screens(mtm);

    // The primary display sits at the global origin; its full height is the flip axis.
    let primary_height = screens
        .iter()
        .find(|s| {
            let o = s.frame().origin;
            o.x == 0.0 && o.y == 0.0
        })
        .or_else(|| screens.iter().next())
        .map_or(0.0, |s| s.frame().size.height);

    let to_ax = |rect: objc2_foundation::NSRect| Frame {
        x: rect.origin.x,
        y: primary_height - (rect.origin.y + rect.size.height),
        width: rect.size.width,
        height: rect.size.height,
    };

    screens
        .iter()
        .map(|screen| Monitor {
            full: to_ax(screen.frame()),
            visible: to_ax(screen.visibleFrame()),
        })
        .collect()
}

/// A +1 CoreFoundation reference, released when it drops.
///
/// CF's rule is that a function with `Create` or `Copy` in its name hands you ownership,
/// so `AXUIElementCopyAttributeValue`, `AXUIElementCreateApplication`, and `AXValueCreate`
/// all return one of these. Wrapping it is what makes the release impossible to forget
/// when a `?` or an early return is added between the call and the end of the function.
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

// SAFETY: the CoreFoundation types this crate owns are `AXUIElement` and `AXValue`, both
// usable from any thread, and `CFRelease` is itself thread-safe. Access to a shared one
// goes through the `Mutex` in `Elements`.
#[expect(unsafe_code)]
unsafe impl Send for Owned {}
// SAFETY: as above.
#[expect(unsafe_code)]
unsafe impl Sync for Owned {}

/// One `AXValue` attribute: the name it is read by, the `AXValueType` it holds, and the
/// Rust type that type means.
///
/// All three together, because `AXValueGetValue` writes through an untyped pointer: an
/// attribute read with the wrong kind, or into the wrong type, is a mismatch nothing would
/// otherwise catch.
trait AxAttribute {
    const NAME: &'static str;
    const KIND: AXValueType;
    type Value: Copy + Default;
}

struct Position;
impl AxAttribute for Position {
    const NAME: &'static str = kAXPositionAttribute;
    const KIND: AXValueType = kAXValueTypeCGPoint;
    type Value = CGPoint;
}

struct Size;
impl AxAttribute for Size {
    const NAME: &'static str = kAXSizeAttribute;
    const KIND: AXValueType = kAXValueTypeCGSize;
    type Value = CGSize;
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

/// Read one `AXValue` attribute of `element`.
fn ax_value<A: AxAttribute>(element: AXUIElementRef) -> Option<A::Value> {
    let value = copy_attribute(element, A::NAME)?;
    let mut out = A::Value::default();
    // SAFETY: `value` is a live `AXValue`, and the impl pairs `A::KIND` with `A::Value`,
    // so a successful read writes an `A::Value` into an `A::Value`.
    #[expect(unsafe_code)]
    let got = unsafe {
        AXValueGetValue(
            value.0.cast_mut().cast(),
            A::KIND,
            std::ptr::from_mut(&mut out).cast(),
        )
    };
    if !got {
        // The attribute did not hold the type it is documented to hold, which is the app's
        // Accessibility implementation misbehaving. Logged rather than fatal: a daemon that
        // remaps the keyboard should not die because some app answered oddly.
        tracing::warn!(
            attribute = A::NAME,
            "an AXValue was not the type it should be"
        );
    }
    got.then_some(out)
}

/// A window's frame, in Accessibility coordinates, or `None` if either half of it
/// cannot be read.
fn window_frame(window: AXUIElementRef) -> Option<Frame> {
    let origin = ax_value::<Position>(window)?;
    let size = ax_value::<Size>(window)?;
    Some(Frame {
        x: origin.x,
        y: origin.y,
        width: size.width,
        height: size.height,
    })
}

/// The focused window of the frontmost app, as a +1 reference the caller releases.
fn focused_window() -> Option<AXUIElementRef> {
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();

    // SAFETY: `pid` names a live process, and `AXUIElementCreateApplication` takes
    // no ownership of it. The returned element is +1 and released below.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid) };

    let attribute = CFString::new(kAXFocusedWindowAttribute);
    let mut window: *const c_void = std::ptr::null();
    // SAFETY: `app` is a live element and `attribute` a live string. On success the
    // out-parameter receives a +1 reference; on failure it is untouched.
    #[expect(unsafe_code)]
    let status = unsafe {
        let s = AXUIElementCopyAttributeValue(
            app,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut window).cast(),
        );
        CFRelease(app.cast());
        s
    };

    (status == 0 && !window.is_null()).then(|| window.cast_mut().cast())
}

/// Set one `AXValue` attribute of `element`.
fn set_attribute<A: AxAttribute>(element: AXUIElementRef, value: A::Value) {
    // SAFETY: `AXValueCreate` copies out of the pointer it is given, which lives for the
    // call, and returns a +1 reference `Owned` takes responsibility for.
    #[expect(unsafe_code)]
    let Some(boxed) =
        (unsafe { Owned::new(AXValueCreate(A::KIND, (&raw const value).cast()).cast()) })
    else {
        return;
    };
    // SAFETY: `element` is live, and setting an attribute takes ownership of neither
    // argument. `boxed` is released when it drops at the end of this function.
    #[expect(unsafe_code)]
    unsafe {
        AXUIElementSetAttributeValue(
            element,
            CFString::new(A::NAME).as_concrete_TypeRef(),
            boxed.0,
        );
    }
}

/// Set the window's position and size, twice.
///
/// Twice because some apps clamp a move against their current size, so the first
/// position lands short of where it was asked to go and the second lands true.
fn set_frame(window: AXUIElementRef, frame: Frame) {
    let origin = CGPoint::new(frame.x, frame.y);
    let size = CGSize::new(frame.width, frame.height);

    for _ in 0..2 {
        set_attribute::<Position>(window, origin);
        set_attribute::<Size>(window, size);
    }
}

#[cfg(test)]
mod tests {
    use super::Frame;

    #[test]
    fn contains_is_half_open() {
        let f = Frame {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        assert!(f.contains(0.0, 0.0));
        assert!(f.contains(99.0, 49.0));
        assert!(!f.contains(100.0, 0.0), "right edge is excluded");
        assert!(!f.contains(0.0, 50.0), "bottom edge is excluded");
        assert!(!f.contains(-1.0, 0.0));
    }

    /// A window's corner picks the monitor it sits on, which is how [`monitor_for`]
    /// chooses the screen to place within. Two monitors side by side, the second
    /// shorter, the way an external display next to a laptop is.
    #[test]
    fn a_point_picks_the_monitor_it_is_on() {
        let left = Frame {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 900.0,
        };
        let right = Frame {
            x: 1600.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        let monitors = [left, right];
        let pick = |x, y| monitors.iter().position(|m| m.contains(x, y));
        assert_eq!(pick(10.0, 10.0), Some(0));
        assert_eq!(pick(1700.0, 10.0), Some(1));
        assert_eq!(pick(3000.0, 10.0), None, "off both monitors");
    }
}

// ---- observation ----

/// What the windows are doing. One variant per thing the observer can tell you.
#[derive(Clone, PartialEq, Debug)]
pub enum WindowChange {
    /// A window appeared, with the frame it appeared at.
    Opened(WindowFrame),
    /// A window moved, with the frame it moved to.
    Moved(WindowFrame),
    /// A window was resized, with the frame it was resized to.
    Resized(WindowFrame),
    /// A window went away.
    Closed(WindowId),
    /// The focused window changed. `None` when the app that came forward has no focused
    /// window, or its window has no readable id.
    Focused(Option<WindowId>),
    /// The monitors changed: one plugged, unplugged, or rearranged.
    Screens(Vec<Monitor>),
}

/// A window and where it is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowFrame {
    pub window: WindowId,
    pub frame: Frame,
}

/// Every window open when the watcher was installed, which one was focused, and the
/// screens they sit on.
///
/// The starting state, for seeding a consumer's model. [`watch`] returns one; the observer
/// reports changes, and at boot nothing has changed yet.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    pub windows: Vec<WindowFrame>,
    pub focused: Option<WindowId>,
    pub screens: Vec<Monitor>,
}

/// What the [`Watcher`] holds, reachable from the callbacks as well as from it.
///
/// Everything but `elements` is main thread only and unlocked: [`watch`], the launch and
/// terminate callbacks, and every `AXObserver` notification all run there.
struct WatcherState {
    /// The one thing shared off the main thread. The [`Watcher`] holds the only strong
    /// reference, so dropping it is what ends a [`WindowSink`]'s access.
    elements: Arc<Elements>,
    /// One entry per observed app. Held here rather than on the [`Watcher`] because the
    /// launch and terminate callbacks are `'static` closures that cannot borrow it.
    apps: RefCell<HashMap<Pid, AppObserver>>,
    on_change: Box<dyn Fn(WindowChange)>,
}

impl WatcherState {
    /// Tell the consumer what happened.
    fn report(&self, change: WindowChange) {
        (self.on_change)(change);
    }

    /// Stop being able to address `window`.
    fn forget(&self, window: WindowId) {
        if let Ok(mut table) = self.elements.0.lock() {
            table.remove(&window);
        }
    }
}

/// One app's observer, and the `refcon` its callbacks reach the [`Watcher`]'s state
/// through.
struct AppObserver {
    observer: AXObserverRef,
    /// The `refcon` every notification for this app carries. Boxed so its address is
    /// stable, and owned here so it is freed exactly when the observer naming it is.
    _registration: Box<Registration>,
}

impl Drop for AppObserver {
    /// Removes the run loop source and releases the observer, in that order: the source
    /// must be gone before the `Registration` that its callbacks dereference is dropped.
    fn drop(&mut self) {
        // SAFETY: `observer` is live and was created by `AXObserverCreate`. Getting its
        // source takes no ownership; removing it and releasing the observer is the
        // documented teardown.
        #[expect(unsafe_code)]
        unsafe {
            let source = AXObserverGetRunLoopSource(self.observer);
            CFRunLoop::get_main().remove_source(
                &CFRunLoopSource::wrap_under_get_rule(source),
                kCFRunLoopDefaultMode,
            );
            CFRelease(self.observer.cast());
        }
    }
}

/// What a notification callback needs: the observer to register a new window on, and the
/// state to report into. A C callback has this instead of a closure.
///
/// `observer` is held rather than a pid, so a window created later is registered without
/// going back through `apps`; nothing in the callback path touches that map.
///
/// [`Weak`](std::rc::Weak), not [`Rc`]: [`WatcherState`] owns `apps`, an [`AppObserver`] owns
/// its registration, so a strong reference here would be a cycle that never frees.
struct Registration {
    observer: AXObserverRef,
    state: std::rc::Weak<WatcherState>,
}

/// The one `AXObserver` callback. `refcon` is the [`Registration`] the app's
/// [`AppObserver`] owns, which is how a C callback reaches the watcher's state without a
/// global.
///
/// Runs on the main thread, since that is the run loop the sources were added to.
#[expect(unsafe_code)]
unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    // SAFETY: `refcon` is the `Box<Registration>` this app's `AppObserver` still owns. The
    // observer's source is removed before the box is dropped, so no notification can
    // arrive after the pointer goes stale.
    let registration = unsafe { &*refcon.cast::<Registration>() };

    // The watcher is gone, so there is nothing to report into.
    let Some(state) = registration.state.upgrade() else {
        return;
    };

    // SAFETY: `notification` is a live string owned by the caller for this call.
    let name = unsafe { CFString::wrap_under_get_rule(notification) }.to_string();

    let name = name.as_str();
    // Comparisons rather than match arms: these constants are lowercase, and a lowercase
    // path in a pattern binds rather than matches the moment it stops resolving.
    if name == kAXWindowCreatedNotification {
        observe_window(&state, registration.observer, refcon, element);
        report_open(&state, element);
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
        if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
            let moved = WindowFrame { window, frame };
            state.report(if name == kAXWindowMovedNotification {
                WindowChange::Moved(moved)
            } else {
                WindowChange::Resized(moved)
            });
        }
    } else if name == kAXUIElementDestroyedNotification {
        if let Some(window) = window_id(element) {
            state.forget(window);
            state.report(WindowChange::Closed(window));
        }
    } else if name == kAXFocusedWindowChangedNotification {
        state.report(WindowChange::Focused(window_id(element)));
    }
}

/// Record a window and subscribe to what it does, without announcing it.
///
/// The setup pass calls this alone: every window it finds is already in the `Snapshot`
/// `watch` returns, so reporting `Opened` for it would be a redundant replay of the seed.
/// A window that opens later goes through `observe_window` too, and `on_notification` then
/// calls `report_open`; see its call site.
///
/// `refcon` is the app's [`Registration`], the same one its own notifications carry: the
/// callback dereferences it whatever fired, so a window registered without it would crash
/// the first time it moved.
fn observe_window(
    state: &WatcherState,
    observer: AXObserverRef,
    refcon: *mut c_void,
    element: AXUIElementRef,
) {
    let Some(window) = window_id(element) else {
        return;
    };

    // SAFETY: `element` is live; retaining it makes the `Owned` below a +1 reference, which
    // is what `Element` releases on drop.
    #[expect(unsafe_code)]
    let retained = unsafe { CFRetain(element.cast()) };
    let Some(owned) = Owned::new(retained) else {
        return;
    };

    for notification in [
        kAXWindowMovedNotification,
        kAXWindowResizedNotification,
        kAXUIElementDestroyedNotification,
    ] {
        add_notification(observer, element, notification, refcon);
    }

    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(window, Arc::new(Element(owned)));
    }
}

/// Report a window as newly open. Its frame is read now, at announce time, rather than
/// carried from `observe_window`: the two are one call apart and the window is live for
/// both. A window whose frame cannot be read is not announced.
fn report_open(state: &WatcherState, element: AXUIElementRef) {
    if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
        state.report(WindowChange::Opened(WindowFrame { window, frame }));
    }
}

/// Subscribe `observer` to one notification on `element`, carrying `refcon`.
///
/// A failure is logged and skipped: an app that will not answer for one notification is
/// still worth observing for the rest.
fn add_notification(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: &str,
    refcon: *mut c_void,
) {
    let name = CFString::new(notification);
    // SAFETY: `observer` and `element` are live, `name` lives for the call, and `refcon` is
    // either null or the stable address of a `Registration` outliving the observer.
    #[expect(unsafe_code)]
    let status =
        unsafe { AXObserverAddNotification(observer, element, name.as_concrete_TypeRef(), refcon) };
    if status != 0 {
        tracing::debug!(notification, status, "could not add a notification");
    }
}

/// Watch one app: its focus changes, its new windows, and every window it already has.
///
/// An app that refuses Accessibility, or has not finished launching, fails
/// `AXObserverCreate`. Logged at `debug` and skipped: its windows are never reported and
/// cannot be addressed, and every other app goes on being observed.
fn observe_app(state: &Rc<WatcherState>, ObservableApp(pid): ObservableApp) {
    if state.apps.borrow().contains_key(&pid) {
        return;
    }

    let mut observer: AXObserverRef = std::ptr::null_mut();
    // SAFETY: `pid` names a process; the out-parameter receives a +1 observer on success
    // and is untouched otherwise.
    #[expect(unsafe_code)]
    let status = unsafe { AXObserverCreate(pid.0, on_notification, &raw mut observer) };
    if status != 0 || observer.is_null() {
        tracing::debug!(?pid, status, "could not observe an app");
        return;
    }

    let registration = Box::new(Registration {
        observer,
        state: Rc::downgrade(state),
    });
    let refcon = std::ptr::from_ref(registration.as_ref()).cast_mut().cast();

    // SAFETY: `pid` names a live process and the element is +1, released with the `Owned`.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid.0) };
    let Some(app) = Owned::new(app.cast()) else {
        return;
    };
    let app_element: AXUIElementRef = app.0.cast_mut().cast();

    for notification in [
        kAXFocusedWindowChangedNotification,
        kAXWindowCreatedNotification,
    ] {
        add_notification(observer, app_element, notification, refcon);
    }

    // SAFETY: `observer` is live; its source is owned by the observer and added at +0.
    #[expect(unsafe_code)]
    unsafe {
        let source = AXObserverGetRunLoopSource(observer);
        CFRunLoop::get_main().add_source(
            &CFRunLoopSource::wrap_under_get_rule(source),
            kCFRunLoopDefaultMode,
        );
    }

    state.apps.borrow_mut().insert(
        pid,
        AppObserver {
            observer,
            _registration: registration,
        },
    );

    for window in app_windows(app_element) {
        observe_window(state, observer, refcon, window.raw());
    }
}

/// Every window an app has right now, each retained.
fn app_windows(app: AXUIElementRef) -> Vec<Element> {
    let Some(value) = copy_attribute(app, kAXWindowsAttribute) else {
        return Vec::new();
    };
    // SAFETY: `kAXWindows` is documented to be a CFArray of AXUIElement, and the array is
    // alive for as long as `value` is.
    #[expect(unsafe_code)]
    let array = unsafe { CFArray::<*const c_void>::wrap_under_get_rule(value.0.cast()) };
    array
        .iter()
        .filter_map(|element| {
            // SAFETY: each entry is a +0 element belonging to the array; retaining it makes
            // the `Owned` a +1 reference.
            #[expect(unsafe_code)]
            let retained = unsafe { CFRetain(*element) };
            Owned::new(retained).map(Element)
        })
        .collect()
}

/// Stop watching an app, reporting every window it took with it.
fn forget_app(state: &WatcherState, pid: Pid) {
    if state.apps.borrow_mut().remove(&pid).is_none() {
        return;
    }
    // The elements the app owned are dead now, and their `AXUIElementDestroyed`
    // notifications went with the observer. Drop them here instead: an app quitting is the
    // reliable end of its windows.
    let gone: Vec<WindowId> = state.elements.0.lock().map_or_else(
        |_| Vec::new(),
        |table| {
            table
                .iter()
                .filter(|(_, element)| window_id(element.raw()).is_none())
                .map(|(id, _)| *id)
                .collect()
        },
    );
    for window in gone {
        state.forget(window);
        state.report(WindowChange::Closed(window));
    }
}

/// One registered notification observer, deregistered when it drops.
///
/// The center is held with the token because deregistering needs the same one that
/// registered: app launches come from `NSWorkspace`'s center and screen changes from the
/// default one.
struct Observation {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    /// Held so the callback outlives the observation. The center copies the block, but the
    /// closure it wraps is ours to keep alive.
    _block: RcBlock<dyn Fn(NonNull<NSNotification>)>,
}

impl Drop for Observation {
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.token).as_ref();
        // SAFETY: `token` is what `addObserverForName...` returned on `center` and is still
        // registered, so this is the documented way to deregister it.
        #[expect(unsafe_code)]
        unsafe {
            self.center.removeObserver(observer);
        }
    }
}

/// Register `on_notification` for `name` on `center`.
fn observe_notification(
    center: &Retained<NSNotificationCenter>,
    name: &NSNotificationName,
    on_notification: impl Fn(&NSNotification) + 'static,
) -> Observation {
    let block = RcBlock::new(move |notif: NonNull<NSNotification>| {
        // SAFETY: Foundation hands the block a valid notification, live for this call.
        #[expect(unsafe_code)]
        let notif = unsafe { notif.as_ref() };
        on_notification(notif);
    });

    // SAFETY: `name` is an immutable extern static. The block is invoked on the main
    // thread, which is where the state it captures lives, and `Observation` owns both the
    // token and the block and deregisters before either is dropped.
    #[expect(unsafe_code)]
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };

    Observation {
        center: center.clone(),
        token,
        _block: block,
    }
}

/// The app a launch or terminate notification is about.
fn notified_app(notif: &NSNotification) -> Option<Retained<NSRunningApplication>> {
    let info = notif.userInfo()?;
    // SAFETY: `NSWorkspaceApplicationKey` is an immutable extern static `NSString` that
    // AppKit initializes before any notification can be delivered.
    #[expect(unsafe_code)]
    let key = unsafe { NSWorkspaceApplicationKey };
    info.objectForKey(key)?
        .downcast::<NSRunningApplication>()
        .ok()
}

/// Watch what the workspace and the screens do: apps coming and going, so a window opened
/// in an app launched later is still reported, and the monitor arrangement changing.
fn watch_notifications(state: &Rc<WatcherState>) -> Vec<Observation> {
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let default = NSNotificationCenter::defaultCenter();
    let mut observations = Vec::new();

    for (name, launched) in [
        // SAFETY: both are immutable extern statics AppKit initializes at startup.
        #[expect(unsafe_code)]
        (unsafe { NSWorkspaceDidLaunchApplicationNotification }, true),
        #[expect(unsafe_code)]
        (
            unsafe { NSWorkspaceDidTerminateApplicationNotification },
            false,
        ),
    ] {
        let state = Rc::downgrade(state);
        observations.push(observe_notification(&workspace, name, move |notif| {
            let (Some(state), Some(app)) = (state.upgrade(), notified_app(notif)) else {
                return;
            };
            if launched {
                // A UI service launching is not something to watch, so there is nothing here
                // to observe.
                if let Some(app) = ObservableApp::of(&app) {
                    observe_app(&state, app);
                }
            } else {
                forget_app(&state, Pid(app.processIdentifier()));
            }
        }));
    }

    let state = Rc::downgrade(state);
    // SAFETY: an immutable extern static AppKit initializes at startup.
    #[expect(unsafe_code)]
    let screens = unsafe { NSApplicationDidChangeScreenParametersNotification };
    observations.push(observe_notification(&default, screens, move |_| {
        // Delivered on the main thread, so reading `NSScreen` here is sound.
        let (Some(state), Some(mtm)) = (state.upgrade(), MainThreadMarker::new()) else {
            return;
        };
        state.report(WindowChange::Screens(read_monitors(mtm)));
    }));

    observations
}

/// Holds every registration that makes windows report. While one of these is alive,
/// changes reach the `on_change` it was built with; dropping it stops them.
///
/// Dropping it is all it takes: `apps` goes, which releases every `AXObserver` and removes
/// its run loop source, and the last strong reference to the element table goes, which is
/// how a live [`WindowSink`] learns it is over. No `Drop` impl needed.
///
/// `!Send`, like `freddie_menu_bar`'s `MenuBar`: it holds main-thread-only state and stays
/// on the thread that built it.
pub struct Watcher {
    /// The workspace and screen observations. Held for their `Drop`, and declared first so
    /// they stop before the state they write into is torn down: fields drop in declaration
    /// order.
    _notifications: Vec<Observation>,
    state: Rc<WatcherState>,
}

impl Watcher {
    /// A handle to perform placements through. Cheap to clone, `Send`, and safe to keep
    /// past the watcher, which it answers [`WindowError::NotWatching`] from.
    #[must_use]
    pub fn sink(&self) -> WindowSink {
        WindowSink {
            elements: Arc::downgrade(&self.state.elements),
        }
    }
}

/// Report every window change to `on_change`, and return the watcher holding the
/// registrations that do it, along with the state before any of them.
///
/// Observes every running app, and every app that launches while the returned [`Watcher`]
/// is alive. Registering is cheap and takes no thread: each `AXObserver` contributes a run
/// loop source to the main run loop, which `freddie_main_loop` is what gets you into.
///
/// `on_change` runs on the main thread, serialized with every other main-thread callback,
/// so it must hand its work elsewhere and return. Sending on a channel is the intended
/// body.
///
/// The [`Snapshot`] comes back with the watcher rather than from a second call, so no
/// caller can let a report land between reading the starting state and using it.
///
/// # Errors
///
/// [`WindowError::NotMainThread`] if called off the main thread, and
/// [`WindowError::NotTrusted`] if Accessibility has not been granted.
pub fn watch(
    on_change: impl Fn(WindowChange) + 'static,
) -> Result<(Watcher, Snapshot), WindowError> {
    let mtm = MainThreadMarker::new().ok_or(WindowError::NotMainThread)?;

    // SAFETY: a plain C predicate over process state; takes no arguments.
    #[expect(unsafe_code)]
    if !unsafe { AXIsProcessTrusted() } {
        return Err(WindowError::NotTrusted);
    }

    let state = Rc::new(WatcherState {
        elements: Arc::new(Elements::default()),
        apps: RefCell::new(HashMap::new()),
        on_change: Box::new(on_change),
    });

    let notifications = watch_notifications(&state);
    for app in NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter_map(|app| ObservableApp::of(&app))
    {
        observe_app(&state, app);
    }

    let screens = read_monitors(mtm);
    let windows = state.elements.0.lock().map_or_else(
        |_| Vec::new(),
        |table| {
            table
                .iter()
                .filter_map(|(window, element)| {
                    window_frame(element.raw()).map(|frame| WindowFrame {
                        window: *window,
                        frame,
                    })
                })
                .collect()
        },
    );
    let snapshot = Snapshot {
        windows,
        focused: focused_window_id(),
        screens,
    };

    tracing::debug!(
        apps = state.apps.borrow().len(),
        windows = snapshot.windows.len(),
        "watching windows"
    );
    Ok((
        Watcher {
            _notifications: notifications,
            state,
        },
        snapshot,
    ))
}

/// The focused window of the frontmost app, by id.
///
/// The one question this crate asks the OS outside a callback. It is the starting value the
/// observer cannot report, because the observer reports changes and none has happened yet.
fn focused_window_id() -> Option<WindowId> {
    let window = focused_window()?;
    let id = window_id(window);
    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    id
}
