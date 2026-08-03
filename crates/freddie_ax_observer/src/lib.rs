//! One `AXObserver` per observable app, kept current as apps launch and quit.
//!
//! The scaffolding a per-app Accessibility watcher stands on: [`watch_apps`] creates an
//! observer for every observable app now running and for every one that launches later, tears
//! it down at termination, and keeps each app's consumer-built registration at a stable
//! address for the life of that app's observer. What to register and what to do with a
//! notification is the consumer's: it brings the C callback, the registration builder, and
//! the per-app install and teardown hooks.
//!
//! Every callback runs on the main thread, from its run loop.
//!
//! macOS only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use accessibility_sys::{
    AXObserverAddNotification, AXObserverCreate, AXObserverGetRunLoopSource, AXObserverRef,
    AXUIElementCreateApplication, AXUIElementRef,
};
use block2::RcBlock;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::runloop::{CFRunLoop, CFRunLoopSource, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};

pub use freddie_windows_types::Pid;

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
/// Built only by [`Self::of`], so an app that has not been vetted cannot be observed.
#[derive(Clone, Copy, Debug)]
pub struct ObservableApp(pub Pid);

impl ObservableApp {
    /// `app` if its windows can be looked at, `None` if it is one of the UI services.
    #[must_use]
    pub fn of(app: &NSRunningApplication) -> Option<Self> {
        (app.activationPolicy() != NSApplicationActivationPolicy::Prohibited)
            .then(|| Self(Pid(app.processIdentifier())))
    }
}

/// One app the watcher can see: the observer to register notifications on, and the app
/// element they are registered against. Borrowed for the duration of one callback.
pub struct AppSeen {
    pub pid: Pid,
    pub observer: AXObserverRef,
    pub app_element: AXUIElementRef,
}

/// The C notification callback a consumer brings: what `AXObserverCreate` takes.
pub type NotificationCallback =
    unsafe extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void);

/// The consumer's per-app install hook: the app seen, and the stable `refcon` its
/// registrations carry.
type OnApp = Box<dyn Fn(&AppSeen, *mut c_void)>;

/// One app's observer, and the `refcon` its callbacks reach the consumer's state through.
struct AppObserver<R> {
    observer: AXObserverRef,
    /// The `refcon` every notification for this app carries. Boxed so its address is
    /// stable, and owned here so it is freed exactly when the observer naming it is.
    _registration: Box<R>,
}

impl<R> Drop for AppObserver<R> {
    /// Removes the run loop source and releases the observer, in that order: the source
    /// must be gone before the registration that its callbacks dereference is dropped.
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

/// What the launch and terminate callbacks reach: the per-app map and the consumer's hooks.
///
/// Main-thread only: [`watch_apps`] and both workspace callbacks run there, so the `RefCell`
/// is never contended.
struct Inner<R> {
    apps: RefCell<HashMap<Pid, AppObserver<R>>>,
    callback: NotificationCallback,
    make_registration: Box<dyn Fn(&AppSeen) -> R>,
    on_app: OnApp,
    on_app_gone: Box<dyn Fn(Pid)>,
}

/// One `AXObserver` per observable app, kept across launches and terminations.
///
/// `R` is the consumer's per-app registration: built once per app, boxed here so its address
/// is stable for the life of that app's observer, handed to the consumer's registrations as
/// the `refcon`, and freed when the observer is released. `!Send`: main thread only, like the
/// window watcher this was extracted from.
pub struct AppWatch<R> {
    /// The workspace observations. Declared first so they stop before the map they write
    /// into is torn down: fields drop in declaration order.
    _notifications: Vec<Observation>,
    _inner: Rc<Inner<R>>,
}

/// Watch one app: create its observer, hand the consumer its hooks.
///
/// An app that refuses Accessibility, or has not finished launching, fails
/// `AXObserverCreate`. Logged at `debug` and skipped: every other app goes on being
/// observed.
fn observe_app<R>(inner: &Rc<Inner<R>>, ObservableApp(pid): ObservableApp) {
    if inner.apps.borrow().contains_key(&pid) {
        return;
    }

    // Before the observer, so the one early return between the two `Create` calls happens
    // while there is still nothing to release.
    // SAFETY: `pid` names a live process and the element is +1, released with the `Owned`.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid.0) };
    let Some(app) = Owned::new(app.cast()) else {
        return;
    };
    let app_element: AXUIElementRef = app.0.cast_mut().cast();

    let mut observer: AXObserverRef = std::ptr::null_mut();
    // SAFETY: `pid` names a process; the out-parameter receives a +1 observer on success
    // and is untouched otherwise.
    #[expect(unsafe_code)]
    let status = unsafe { AXObserverCreate(pid.0, inner.callback, &raw mut observer) };
    if status != 0 || observer.is_null() {
        tracing::debug!(?pid, status, "could not observe an app");
        return;
    }

    let seen = AppSeen {
        pid,
        observer,
        app_element,
    };
    let registration = Box::new((inner.make_registration)(&seen));
    let refcon = std::ptr::from_ref(registration.as_ref()).cast_mut().cast();

    // SAFETY: `observer` is live; its source is owned by the observer and added at +0.
    #[expect(unsafe_code)]
    unsafe {
        let source = AXObserverGetRunLoopSource(observer);
        CFRunLoop::get_main().add_source(
            &CFRunLoopSource::wrap_under_get_rule(source),
            kCFRunLoopDefaultMode,
        );
    }

    inner.apps.borrow_mut().insert(
        pid,
        AppObserver {
            observer,
            _registration: registration,
        },
    );

    // After the insert, so a consumer hook that fires a notification synchronously finds
    // the app already observed.
    (inner.on_app)(&seen, refcon);
}

/// Stop watching an app.
///
/// The observer is dropped before `on_app_gone` runs: that removes its run loop source so a
/// late notification cannot run against a registration that no longer exists.
fn forget_app<R>(inner: &Inner<R>, pid: Pid) {
    if inner.apps.borrow_mut().remove(&pid).is_none() {
        return;
    }
    (inner.on_app_gone)(pid);
}

/// Observe every running observable app now and every one that launches later.
///
/// `callback` is the consumer's C notification callback (its `refcon` is the `&R` for that
/// app). `on_app` runs once per observed app — at install for the running set, at launch for
/// the rest — and is where the consumer registers its notifications and seeds; it receives
/// the stable `refcon` pointer for those registrations. `on_app_gone` runs after the app's
/// observer and registration are torn down.
pub fn watch_apps<R: 'static>(
    callback: NotificationCallback,
    make_registration: impl Fn(&AppSeen) -> R + 'static,
    on_app: impl Fn(&AppSeen, *mut c_void) + 'static,
    on_app_gone: impl Fn(Pid) + 'static,
) -> AppWatch<R> {
    let inner = Rc::new(Inner {
        apps: RefCell::new(HashMap::new()),
        callback,
        make_registration: Box::new(make_registration),
        on_app: Box::new(on_app),
        on_app_gone: Box::new(on_app_gone),
    });

    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let mut notifications = Vec::new();
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
        let inner = Rc::downgrade(&inner);
        notifications.push(observe_notification(&workspace, name, move |notif| {
            let (Some(inner), Some(app)) = (inner.upgrade(), notified_app(notif)) else {
                return;
            };
            if launched {
                // ObservableApp::of drops UI services: they have no windows worth watching.
                if let Some(app) = ObservableApp::of(&app) {
                    observe_app(&inner, app);
                }
            } else {
                forget_app(&inner, Pid(app.processIdentifier()));
            }
        }));
    }

    for app in NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter_map(|app| ObservableApp::of(&app))
    {
        observe_app(&inner, app);
    }

    tracing::debug!(apps = inner.apps.borrow().len(), "observing apps");
    AppWatch {
        _notifications: notifications,
        _inner: inner,
    }
}

/// Subscribe `observer` to one notification on `element`, carrying `refcon`.
///
/// A failure is logged and skipped: an app that will not answer for one notification is
/// still worth observing for the rest.
///
/// # Safety
///
/// `observer` and `element` must be live, and `refcon` must be null or a pointer that stays
/// valid for as long as the observer can deliver — the boxed registration [`watch_apps`]
/// hands its hooks qualifies.
#[expect(unsafe_code)]
pub unsafe fn add_notification(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: &str,
    refcon: *mut c_void,
) {
    let name = CFString::new(notification);
    // SAFETY: the caller's contract, plus `name` living for the call.
    let status =
        unsafe { AXObserverAddNotification(observer, element, name.as_concrete_TypeRef(), refcon) };
    if status != 0 {
        tracing::debug!(notification, status, "could not add a notification");
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

/// One registered notification observer, deregistered when it drops.
///
/// The center is held with the token because deregistering needs the same one that
/// registered: app launches come from `NSWorkspace`'s center and screen changes from the
/// default one.
pub struct Observation {
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
pub fn observe_notification(
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
#[must_use]
pub fn notified_app(notif: &NSNotification) -> Option<Retained<NSRunningApplication>> {
    let info = notif.userInfo()?;
    // SAFETY: `NSWorkspaceApplicationKey` is an immutable extern static `NSString` that
    // AppKit initializes before any notification can be delivered.
    #[expect(unsafe_code)]
    let key = unsafe { NSWorkspaceApplicationKey };
    info.objectForKey(key)?
        .downcast::<NSRunningApplication>()
        .ok()
}
