//! Watching the windows on screen, and moving them.
//!
//! The shape `freddie_app_nav` has: a source, a sink, and a seed.
//!
//! - [`watch`] is the source. One `AXObserver` per app reports windows opening, moving,
//!   resizing, and closing, and the frontmost app's focused window changing. `NSWorkspace`
//!   observers keep the observed set current as apps launch and quit and report the focused
//!   window afresh on every app activation, and a screen observer reports the monitors. Every
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
//! `CGWindow` can read geometry but not write it. A placement is queued on the main thread and
//! written on a thread of its own, because the write costs tens of milliseconds and main is
//! what every other source is waiting on.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use accessibility_sys::{
    AXError, AXIsProcessTrusted, AXObserverRef, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetAttributeValue, AXValueCreate,
    AXValueType, kAXFocusedWindowAttribute, kAXFocusedWindowChangedNotification,
    kAXPositionAttribute, kAXSizeAttribute, kAXUIElementDestroyedNotification, kAXValueTypeCGPoint,
    kAXValueTypeCGSize, kAXWindowCreatedNotification, kAXWindowMovedNotification,
    kAXWindowResizedNotification, kAXWindowsAttribute, pid_t,
};
use core_foundation::array::CFArray;
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::CGRect;
use core_graphics::geometry::{CGPoint, CGSize};
use core_graphics::window::{
    CGWindowID, copy_window_info, kCGNullWindowID, kCGWindowBounds,
    kCGWindowListOptionIncludingWindow,
};
use freddie_ax_observer::{
    AppSeen, AppWatch, ObservableApp, Observation, add_notification, notified_app,
    observe_notification, watch_apps,
};
use freddie_main_loop::{MainWaker, WakingSender};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSScreen, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{MainThreadMarker, NSNotificationCenter};

pub use freddie_windows_types::{
    Frame, Monitor, Pid, Placement, WindowChange, WindowError, WindowId,
};

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
    const fn raw(&self) -> AXUIElementRef {
        self.0.0.cast_mut().cast()
    }

    /// A second owned reference to the same element, for handing to another thread.
    ///
    /// `CFRetain` rather than deriving `Clone` on [`Owned`], which two values naming one element
    /// would release twice.
    fn retained(&self) -> Self {
        // SAFETY: `self` holds a live +1 reference, so retaining it yields a second one, which the
        // returned `Element` releases on drop.
        #[expect(unsafe_code)]
        let raw = unsafe { CFRetain(self.raw().cast()) };
        Self(Owned(raw))
    }
}

/// A window being watched: the element to address it through, and the app it belongs to.
struct Watched {
    element: Element,
    /// Which app registered it, so [`watch`]'s app-gone hook can report every window the
    /// app took with it.
    pid: Pid,
}

/// Every window that can be addressed, the element to address it through, and where it is.
///
/// Main-thread only, like `apps`: the AX callbacks that write it and the `pump` that reads it both
/// run there, so there is nothing to lock.
type Elements = HashMap<WindowId, Watched>;

/// The handle a placement is performed through.
///
/// Cheap to clone and unattached to the thread that made it: it is a sender, and the placement is
/// looked up and performed by the thread that owns the table.
#[derive(Clone)]
pub struct WindowSink {
    placements: WakingSender<Placement>,
}

impl WindowSink {
    /// Move and resize one window: `target` names which, and the rectangle it goes to.
    ///
    /// Queues the placement and wakes the main thread, which owns the element table. The write
    /// costs tens of milliseconds and runs on a thread of its own, so this returns immediately and
    /// a caller on a latency-sensitive loop needs no thread of its own.
    ///
    /// The frame is the caller's, already worked out. This does not consult the screen, the
    /// frontmost app, or anything else.
    ///
    /// # Errors
    ///
    /// [`WindowError::NotWatching`] if the watcher has been dropped. A window that is not being
    /// observed cannot be reported here, because the lookup happens after the send;
    /// [`Watcher::pump`] logs it at `debug` instead.
    pub fn set_frame(&self, placement: Placement) -> Result<(), WindowError> {
        self.placements
            .send(placement)
            .map_err(|_| WindowError::NotWatching)
    }
}

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
// usable from any thread, and `CFRelease` is itself thread-safe. An element is moved to the
// placement thread rather than shared.
#[expect(unsafe_code)]
unsafe impl Send for Owned {}

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

/// The pid of the frontmost application, if there is one.
fn frontmost_pid() -> Option<pid_t> {
    Some(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier(),
    )
}

/// Whether `pid` names the frontmost application right now.
/// The focused window of the app with pid `pid`, as a +1 reference the caller releases.
fn focused_window(pid: pid_t) -> Option<AXUIElementRef> {
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
///
/// A refusal is logged and skipped rather than returned: a placement is two or three of these and
/// there is nothing useful for a caller to do with a partial one. The log is what says whether a
/// write landed, which is how the ordering in [`set_frame`] is checked.
///
/// `warn`, because a window that does not go where it was asked to go is visible to whoever asked.
/// An app refusing a frame it considers out of bounds, or below its minimum size, is the likeliest
/// reason a placement looks broken, and it should not take `--level debug` to find out.
fn set_attribute<A: AxAttribute>(element: AXUIElementRef, value: A::Value) {
    // SAFETY: `AXValueCreate` copies out of the pointer it is given, which lives for the
    // call, and returns a +1 reference `Owned` takes responsibility for.
    #[expect(unsafe_code)]
    let Some(boxed) =
        (unsafe { Owned::new(AXValueCreate(A::KIND, (&raw const value).cast()).cast()) })
    else {
        tracing::warn!(attribute = A::NAME, "could not box an attribute value");
        return;
    };
    // SAFETY: `element` is live, and setting an attribute takes ownership of neither
    // argument. `boxed` is released when it drops at the end of this function.
    #[expect(unsafe_code)]
    let status = unsafe {
        AXUIElementSetAttributeValue(
            element,
            CFString::new(A::NAME).as_concrete_TypeRef(),
            boxed.0,
        )
    };
    if status != 0 {
        tracing::warn!(
            attribute = A::NAME,
            status,
            "an attribute write was refused"
        );
    }
}

/// A width and a height.
///
/// Not `CGSize`, which does not implement `PartialEq`, and deliberately without CoreGraphics in it
/// so the write ordering is arithmetic a test can table.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Extent {
    width: f64,
    height: f64,
}

/// The size writes one placement performs, around the move that sits between them.
///
/// The move is unconditional and always goes to the target's origin, so it is not named here.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Writes {
    /// The size to shrink to before moving, when either axis shrinks.
    shrink: Option<Extent>,
    /// The size to grow to after moving, when either axis grows.
    grow: Option<Extent>,
}

/// Shrink, move, grow.
///
/// Position and size are separate writes and an app validates each against the value the other one
/// holds, so the intermediate between two writes has to fit as well as the endpoints do. Shrinking
/// first keeps the intermediate inside `from`, which the window already occupies. Moving at the
/// shrunk size keeps it inside `to` on both axes. Growing happens once the origin is already right,
/// so the last write is `to` itself. Nothing here consults a screen, because containment in `from`
/// or `to` is what makes each step safe and both of those fit by construction.
fn writes_for(from: Frame, to: Frame) -> Writes {
    let shrunk = Extent {
        width: from.width.min(to.width),
        height: from.height.min(to.height),
    };
    let target = Extent {
        width: to.width,
        height: to.height,
    };
    Writes {
        shrink: (shrunk.width < from.width || shrunk.height < from.height).then_some(shrunk),
        grow: (target.width > shrunk.width || target.height > shrunk.height).then_some(target),
    }
}

/// Move and resize one window, in an order that cannot be clamped. See [`writes_for`].
///
/// Two writes for a pure shrink or a pure grow, three when one axis goes each way. A stale `from`
/// cannot break it: too small under-shrinks and every later step is still bounded by `to`, and too
/// large makes the first write a grow that an app may clamp, which only leaves the window smaller
/// than asked. The two writes that must not be clamped, the move and the final size, are bounded by
/// `to` either way.
fn set_frame(window: AXUIElementRef, from: Frame, to: Frame) {
    let Writes { shrink, grow } = writes_for(from, to);
    if let Some(extent) = shrink {
        set_attribute::<Size>(window, CGSize::new(extent.width, extent.height));
    }
    set_attribute::<Position>(window, CGPoint::new(to.x, to.y));
    if let Some(extent) = grow {
        set_attribute::<Size>(window, CGSize::new(extent.width, extent.height));
    }
}

// ---- observation ----

/// What the [`Watcher`] holds, reachable from the callbacks as well as from it.
///
/// Main-thread only: [`watch`], the launch and terminate callbacks, every `AXObserver`
/// notification, and [`Watcher::pump`] all run there.
struct WatcherState {
    /// Every window being watched. A `RefCell` and not a `Mutex`: nothing off the main thread
    /// reaches it.
    elements: RefCell<Elements>,
    on_change: Box<dyn Fn(WindowChange)>,
}

impl WatcherState {
    /// Tell the consumer what happened.
    fn report(&self, change: WindowChange) {
        (self.on_change)(change);
    }

    /// Stop being able to address `window`. Whether there was an entry to remove, which is
    /// whether this is the report that closes it: a window's own `AXUIElementDestroyed` and
    /// its app terminating both arrive, in either order, and only the first of them reports.
    fn forget(&self, window: WindowId) -> bool {
        self.elements.borrow_mut().remove(&window).is_some()
    }

    /// Forget whichever window `element` names, and say which it was.
    ///
    /// By identity rather than by id: `kAXUIElementDestroyed` arrives for an element the app has
    /// already torn down, `_AXUIElementGetWindow` refuses it, and `CFEqual` still matches the
    /// element that was retained when the window opened. `None` when the element was not a window
    /// this was watching, which is most of them: the notification is registered on the app, so it
    /// reports every element the app destroys.
    fn forget_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        let mut table = self.elements.borrow_mut();
        // SAFETY: both are live `AXUIElement`s as far as CoreFoundation is concerned. A destroyed
        // element is still a valid CF object; it is the Accessibility calls on it that fail.
        #[expect(unsafe_code)]
        let found = table
            .iter()
            .find(|(_, held)| unsafe { CFEqual(held.element.raw().cast(), element.cast()) != 0 })
            .map(|(id, _)| *id)?;
        table.remove(&found);
        Some(found)
    }
}

/// What a notification callback needs: the observer to register a new window on, the pid of
/// the app it is for, and the state to report into. A C callback has this instead of a closure.
///
/// `observer` is held so a window created later is registered without a lookup. `pid` is
/// which app a notification is about, which is what a focus report names and which app a new
/// window belongs to.
///
/// [`Weak`](std::rc::Weak), not [`Rc`]: the [`AppWatch`] owns the registration and outlives
/// nothing the [`Watcher`] does not also own, so the weak reference adds no lifetime and a
/// stale callback upgrades to nothing instead of keeping the state alive.
struct Registration {
    observer: AXObserverRef,
    pid: Pid,
    state: std::rc::Weak<WatcherState>,
}

/// The one `AXObserver` callback. `refcon` is a [`Registration`] the app's [`AppObserver`]
/// owns, which is how a C callback reaches the watcher's state without a global.
///
/// Runs on the main thread, since that is the run loop the sources were added to.
#[expect(unsafe_code)]
unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    // Copy every `Copy` field out and end the `&Registration` borrow before any branch runs.
    let (state, observer, pid, refcon) = {
        // SAFETY: `refcon` is the `Box<Registration>` this app's `AppObserver` still owns. The
        // observer's source is removed before the box is dropped, so no notification can arrive
        // after the pointer goes stale.
        let registration = unsafe { &*refcon.cast::<Registration>() };
        let Some(state) = registration.state.upgrade() else {
            return;
        };
        (state, registration.observer, registration.pid, refcon)
    };

    // SAFETY: `notification` is a live string owned by the caller for this call.
    let name = unsafe { CFString::wrap_under_get_rule(notification) }.to_string();

    let name = name.as_str();
    // Comparisons rather than match arms: these constants are lowercase, and a lowercase
    // path in a pattern binds rather than matches the moment it stops resolving.
    if name == kAXWindowCreatedNotification {
        observe_window(&state, observer, pid, refcon, element);
        if let Some(window) = window_id(element) {
            state.report(WindowChange::Opened(window));
        }
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
        if let Some(window) = window_id(element) {
            state.report(if name == kAXWindowMovedNotification {
                WindowChange::Moved(window)
            } else {
                WindowChange::Resized(window)
            });
        }
    } else if name == kAXUIElementDestroyedNotification {
        // Registered on the app, so this reports every element the app destroys. The element
        // cannot be asked for its id, and `CFEqual` still matches the one retained when the
        // window opened, so the table answers instead. `None` for anything that was not a
        // window being watched.
        if let Some(window) = state.forget_element(element) {
            state.report(WindowChange::Closed(window));
        }
    } else if name == kAXFocusedWindowChangedNotification {
        state.report(WindowChange::FocusChanged(pid));
    }
}

/// Record a window and subscribe to its moves and resizes. Nothing is announced here.
///
/// The setup pass calls this alone: every window it finds is already in the `Snapshot` `watch`
/// returns, so reporting `Opened` for it would be a redundant replay of the seed. A window that
/// opens later goes through here too, and `on_notification` then calls `report_open`; see its
/// call site.
///
/// Destroy is registered on the app element, not here: a window-element registration for
/// `kAXUIElementDestroyed` returns success and never fires. Move and resize keep the app's
/// `refcon`; the live element still answers for its id.
fn observe_window(
    state: &WatcherState,
    observer: AXObserverRef,
    pid: Pid,
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

    for notification in [kAXWindowMovedNotification, kAXWindowResizedNotification] {
        // SAFETY: `observer` and `element` are live, and `refcon` is the boxed registration
        // the app's observer owns, freed only after the observer stops delivering.
        #[expect(unsafe_code)]
        unsafe {
            add_notification(observer, element, notification, refcon);
        }
    }

    state.elements.borrow_mut().insert(
        window,
        Watched {
            element: Element(owned),
            pid,
        },
    );
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

/// Watch what activation and the screens do: the frontmost app changing, which posts no
/// focus-changed notification of its own, and the monitor arrangement changing. App launches
/// and terminations are [`watch_apps`]'s to observe.
fn watch_notifications(state: &Rc<WatcherState>) -> Vec<Observation> {
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let default = NSNotificationCenter::defaultCenter();
    let mut observations = Vec::new();

    let activation_state = Rc::downgrade(state);
    // SAFETY: an immutable extern static AppKit initializes at startup.
    #[expect(unsafe_code)]
    let activated = unsafe { NSWorkspaceDidActivateApplicationNotification };
    observations.push(observe_notification(&workspace, activated, move |notif| {
        let (Some(state), Some(app)) = (activation_state.upgrade(), notified_app(notif)) else {
            return;
        };
        // App activation posts no focus-changed notification, so the newly frontmost app's
        // focused window is read here. A UI service that can never be frontmost is skipped
        // the same way the observer never watches its windows.
        if let Some(ObservableApp(pid)) = ObservableApp::of(&app) {
            state.report(WindowChange::FocusChanged(pid));
        }
    }));

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
/// Dropping it is all it takes: the [`AppWatch`] goes, which releases every `AXObserver` and
/// removes its run loop source, and the placement receiver goes, which is how a live
/// [`WindowSink`] learns it is over. No `Drop` impl needed.
///
/// `!Send`, like `freddie_menu_bar`'s `MenuBar`: it holds main-thread-only state and stays
/// on the thread that built it.
pub struct Watcher {
    /// The activation and screen observations. Held for their `Drop`, and declared first so
    /// they stop before the state they write into is torn down: fields drop in declaration
    /// order.
    _notifications: Vec<Observation>,
    /// The per-app observers, kept current as apps launch and quit. Declared before `state`
    /// for the same reason as the observations.
    _apps: AppWatch<Registration>,
    /// Handed to every [`WindowSink`].
    placements_sender: WakingSender<Placement>,
    /// Placements waiting to be performed. Drained by [`Self::pump`] on the main thread.
    placements: Receiver<Placement>,
    state: Rc<WatcherState>,
}

impl Watcher {
    /// A handle to perform placements through. Cheap to clone, `Send`, and safe to keep past the
    /// watcher, which it answers [`WindowError::NotWatching`] from.
    #[must_use]
    pub fn sink(&self) -> WindowSink {
        WindowSink {
            placements: self.placements_sender.clone(),
        }
    }

    /// Perform every placement queued since the last wake.
    ///
    /// On the main thread, because that is where the element table lives. The lookup is a hashmap
    /// hit; the write is handed to a thread of its own, because it costs tens of milliseconds and
    /// this thread is what every other source is waiting on.
    pub fn pump(&self) {
        for placement in self.placements.try_iter() {
            let found = self
                .state
                .elements
                .borrow()
                .get(&placement.window)
                .map(|watched| watched.element.retained());
            let Some(element) = found else {
                tracing::debug!(?placement, "no such window to place");
                continue;
            };
            std::thread::spawn(move || {
                set_frame(element.raw(), placement.from, placement.to);
                tracing::debug!(?placement, "set a window's frame");
            });
        }
    }
}

/// Report every window change to `on_change`, returning the watcher that does it.
///
/// The install pass reports through `on_change` too — one `Screens`, one `Opened` per existing
/// window, one `FocusChanged` for the frontmost app — so the seed path and the steady-state
/// path are the same code, and a consumer's model starts empty.
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
    waker: &MainWaker,
    on_change: impl Fn(WindowChange) + 'static,
) -> Result<Watcher, WindowError> {
    let mtm = MainThreadMarker::new().ok_or(WindowError::NotMainThread)?;

    // SAFETY: a plain C predicate over process state; takes no arguments.
    #[expect(unsafe_code)]
    if !unsafe { AXIsProcessTrusted() } {
        return Err(WindowError::NotTrusted);
    }

    let state = Rc::new(WatcherState {
        elements: RefCell::new(HashMap::new()),
        on_change: Box::new(on_change),
    });
    let (placements_sender, placements) = waker.channel::<Placement>();

    let notifications = watch_notifications(&state);
    let apps = {
        let registration_state = Rc::downgrade(&state);
        let install_state = Rc::clone(&state);
        let gone_state = Rc::clone(&state);
        watch_apps(
            on_notification,
            move |seen| Registration {
                observer: seen.observer,
                pid: seen.pid,
                state: registration_state.clone(),
            },
            move |seen: &AppSeen, refcon| {
                for notification in [
                    kAXFocusedWindowChangedNotification,
                    kAXWindowCreatedNotification,
                    // On the app element, not on each window: a window-element registration
                    // for this one returns success and never fires.
                    kAXUIElementDestroyedNotification,
                ] {
                    // SAFETY: `seen` is live for this callback, and `refcon` is the boxed
                    // registration the app's observer owns, freed only after the observer
                    // stops delivering.
                    #[expect(unsafe_code)]
                    unsafe {
                        add_notification(seen.observer, seen.app_element, notification, refcon);
                    }
                }
                for window in app_windows(seen.app_element) {
                    observe_window(
                        &install_state,
                        seen.observer,
                        seen.pid,
                        refcon,
                        window.raw(),
                    );
                }
            },
            // The app is gone: report every window it took with it, then the app itself. A
            // window whose destroy notification already arrived was forgotten then; only the
            // ones still in the table report here.
            move |pid| {
                let windows: Vec<WindowId> = gone_state
                    .elements
                    .borrow()
                    .iter()
                    .filter(|(_, watched)| watched.pid == pid)
                    .map(|(id, _)| *id)
                    .collect();
                for window in windows {
                    if gone_state.forget(window) {
                        gone_state.report(WindowChange::Closed(window));
                    }
                }
                gone_state.report(WindowChange::AppGone(pid));
            },
        )
    };

    state.report(WindowChange::Screens(read_monitors(mtm)));
    let windows: Vec<WindowId> = state.elements.borrow().keys().copied().collect();
    for window in &windows {
        state.report(WindowChange::Opened(*window));
    }
    if let Some(pid) = frontmost_pid() {
        state.report(WindowChange::FocusChanged(Pid(pid)));
    }

    tracing::debug!(windows = windows.len(), "watching windows");
    Ok(Watcher {
        _notifications: notifications,
        _apps: apps,
        placements_sender,
        placements,
        state,
    })
}

/// The focused window of `pid` right now, through Accessibility.
///
/// The elements it creates are its own and per call. `None` when the app has no focused
/// window, its window has no readable id, or it will not answer. Callable from any thread; an
/// effect performer's read.
#[must_use]
pub fn focused_window_of(pid: Pid) -> Option<WindowId> {
    focused_window_id(pid.0)
}

/// The frame of `window` right now, by id through the window server. `None` for a window that
/// is gone or unreadable. Callable from any thread; an effect performer's read.
#[must_use]
pub fn frame_of(window: WindowId) -> Option<Frame> {
    let infos = copy_window_info(kCGWindowListOptionIncludingWindow, window.0)?;
    let raw = infos.get(0)?;
    // SAFETY: the array holds window-info dictionaries per `CGWindowListCopyWindowInfo`'s
    // contract, and `kCGWindowBounds` is an immutable extern static the framework provides.
    // Both wraps retain, so each releases exactly what it holds.
    #[expect(unsafe_code)]
    let bounds = unsafe {
        let info = CFDictionary::<*const c_void, *const c_void>::wrap_under_get_rule((*raw).cast());
        let bounds = *info.find(kCGWindowBounds.cast::<c_void>())?;
        CFDictionary::wrap_under_get_rule(bounds.cast())
    };
    let rect = CGRect::from_dict_representation(&bounds)?;
    Some(Frame {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    })
}

/// The focused window of the app with pid `pid`, by id.
///
/// Read at boot for the frontmost app, seeding the value the observer cannot report because
/// none has changed yet, and read again on each app activation, which posts no focus-changed
/// notification of its own.
fn focused_window_id(pid: pid_t) -> Option<WindowId> {
    let window = focused_window(pid)?;
    let id = window_id(window);
    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    id
}

#[cfg(test)]
mod tests {
    use super::{Extent, Frame, Writes, writes_for};

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

    const FROM: Frame = Frame {
        x: 1000.0,
        y: 100.0,
        width: 600.0,
        height: 400.0,
    };

    const fn extent(width: f64, height: f64) -> Extent {
        Extent { width, height }
    }

    // Growing while moving left: nothing to shrink, so the move goes first at the old size and the
    // grow lands at the target origin.
    #[test]
    fn a_pure_grow_moves_before_it_grows() {
        let to = Frame {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 900.0,
        };
        assert_eq!(
            writes_for(FROM, to),
            Writes {
                shrink: None,
                grow: Some(extent(1600.0, 900.0))
            }
        );
    }

    // Shrinking while moving right: the shrink goes first, so the intermediate never reaches past
    // the target's right edge.
    #[test]
    fn a_pure_shrink_shrinks_before_it_moves() {
        let to = Frame {
            x: 1400.0,
            y: 100.0,
            width: 400.0,
            height: 300.0,
        };
        assert_eq!(
            writes_for(FROM, to),
            Writes {
                shrink: Some(extent(400.0, 300.0)),
                grow: None
            }
        );
    }

    // One axis each way: both size writes happen, and the first shrinks only the axis that shrinks.
    #[test]
    fn a_mixed_change_shrinks_then_grows() {
        let to = Frame {
            x: 500.0,
            y: 100.0,
            width: 400.0,
            height: 900.0,
        };
        assert_eq!(
            writes_for(FROM, to),
            Writes {
                shrink: Some(extent(400.0, 400.0)),
                grow: Some(extent(400.0, 900.0)),
            }
        );
    }

    // A frame that is already the right size is one write, and it is the move.
    #[test]
    fn an_unchanged_size_is_only_a_move() {
        let to = Frame {
            x: 0.0,
            y: 0.0,
            ..FROM
        };
        assert_eq!(
            writes_for(FROM, to),
            Writes {
                shrink: None,
                grow: None
            }
        );
    }

    // The invariant the order rests on: the shrink never exceeds `from` and the size the move
    // happens at never exceeds `to`, on both axes, which is why no screen is consulted.
    #[test]
    fn no_intermediate_exceeds_its_endpoint() {
        for to in [
            Frame {
                x: 0.0,
                y: 0.0,
                width: 1600.0,
                height: 900.0,
            },
            Frame {
                x: 1400.0,
                y: 100.0,
                width: 400.0,
                height: 300.0,
            },
            Frame {
                x: 500.0,
                y: 100.0,
                width: 400.0,
                height: 900.0,
            },
            Frame {
                x: 0.0,
                y: 0.0,
                ..FROM
            },
        ] {
            let writes = writes_for(FROM, to);
            if let Some(shrink) = writes.shrink {
                assert!(shrink.width <= FROM.width && shrink.height <= FROM.height);
            }
            let moved = writes.shrink.unwrap_or(extent(FROM.width, FROM.height));
            assert!(moved.width <= to.width && moved.height <= to.height);
        }
    }
}
