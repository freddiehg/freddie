# Window observation

`freddie_windows` becomes a source as well as a sink, the shape `freddie_app_nav` already has: [`watch`] reports what windows are doing and hands back the starting state, `WindowSink::set_frame` asks for a change, and nothing ties a call to a report.

Mercury's model ends up holding the windows: every window's id and frame, which one is focused, and the monitors. Events fill it, the way they fill `foreground` and the Chrome tab URL, so dispatch reads no OS state.

Nothing consumes the table when this doc is done. `Place(Placement)` still works exactly as it does now. `refactors/pending/placement-in-the-model.md` is what makes the placement path use it, and `refactors/pending/window-restore.md` is what adds `r`.

## What is observed

Per app, one `AXObserver` on the application element, created when the app appears and released when it quits. On the application element:

- `kAXFocusedWindowChangedNotification`
- `kAXWindowCreatedNotification`

On each window element, added as the window is seen:

- `kAXWindowMovedNotification` and `kAXWindowResizedNotification`, one variant each. A consumer that treats them alike collapses them itself.
- `kAXUIElementDestroyedNotification`.

An `AXObserver` gives a `CFRunLoopSource`. It is added to the main run loop, which `freddie_main_loop` is already inside, so a callback runs there exactly as `freddie_app_nav`'s does. Observation is per-pid and costs no thread and no poll.

An app that refuses Accessibility, or has not finished launching, fails `AXObserverCreate`. That is logged at `debug` and the app is skipped: its windows are never reported and cannot be addressed, and every other app goes on being observed.

## Identity

A window's identity is its `CGWindowID`. An `AXUIElementRef` is not it: elements are created per call and two for the same window are different pointers.

`_AXUIElementGetWindow` is the only way across. It is private, exported by HIServices, and has been there since 10.x. A window whose id cannot be read produces no events.

The crate keeps the reverse direction too: a table from `WindowId` to the retained element observing it, maintained by the observer and read on the main thread when a placement is performed. Without it, addressing a window would mean walking every app's `kAXWindowsAttribute` to find the one with a matching id. It is the only place that mapping exists, so the model and the effects speak `WindowId` alone.

---

# Change 1: read a window's whole frame

`place` reads only the position today, to pick a monitor. Everything downstream needs the size too, and both come from the same shaped call. No behavior change.

Before, in `crates/freddie_windows/src/lib.rs`:

```rust
/// The focused window's top-left corner, in Accessibility coordinates, or `None` if
/// it cannot be read.
fn window_origin(window: AXUIElementRef) -> Option<CGPoint> {
    let attribute = CFString::new(kAXPositionAttribute);
    let mut value: *const c_void = std::ptr::null();
    // SAFETY: `window` is a live element and `attribute` a live string. On success
    // the out-parameter receives a +1 `AXValue`; on failure it is untouched.
    #[expect(unsafe_code)]
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            window,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }

    let mut point = CGPoint::new(0.0, 0.0);
    // SAFETY: `value` is a +1 `AXValue` of CGPoint type; `AXValueGetValue` copies it
    // into `point`. The value is released afterward.
    #[expect(unsafe_code)]
    let got = unsafe {
        let ok = AXValueGetValue(
            value.cast_mut().cast(),
            kAXValueTypeCGPoint,
            std::ptr::from_mut(&mut point).cast(),
        );
        CFRelease(value);
        ok
    };
    got.then_some(point)
}
```

After:

```rust
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
        tracing::warn!(attribute = A::NAME, "an AXValue was not the type it should be");
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
```

`monitor_for` takes the frame its caller already read, so `place` reads it once.

Before:

```rust
fn monitor_for(window: AXUIElementRef) -> Option<Monitor> {
    let monitors = MONITORS.read().ok()?.clone();
    let first = *monitors.first()?;
    let chosen = window_origin(window)
        .and_then(|p| monitors.iter().find(|m| m.full.contains(p.x, p.y)).copied())
        .unwrap_or(first);
    Some(chosen)
}
```

After:

```rust
/// The monitor a window is on: the one whose full frame contains the window's top-left
/// corner, or the first monitor if none does or the frame could not be read. `None` only
/// if [`init`] never cached any monitor.
fn monitor_for(frame: Option<Frame>) -> Option<Monitor> {
    let monitors = MONITORS.read().ok()?.clone();
    let first = *monitors.first()?;
    let chosen = frame
        .and_then(|f| monitors.iter().find(|m| m.full.contains(f.x, f.y)).copied())
        .unwrap_or(first);
    Some(chosen)
}
```

`place`'s one changed line:

```rust
    let monitor = monitor_for(window_frame(window)).ok_or(WindowError::NotInitialized)?;
```

`Frame` and `Monitor` become `pub`, with `Monitor`'s fields public, since the model will hold them. `Frame` gains nothing else here.

Imports gained: `kAXValueTypeCGSize` and `AXValueType`.

# Change 2: a window's id

No behavior change: `place` logs the id it read.

```rust
use core_graphics::window::{CGWindowID, kCGNullWindowID};

/// A window's `CGWindowID`: the identity that outlives any one `AXUIElement` naming it.
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
```

In `place`, before the `CFRelease`, since it reads the element:

```rust
    tracing::debug!(?placement, ?target, id = ?window_id(window), "placed the focused window");
```

# Change 3: the write direction uses the same pairing

`set_frame` boxes a `CGPoint` and a `CGSize` with `AXValueCreate` and releases both by hand, twice per call. [`AxAttribute`] already knows the name and the kind for each, so the same trait serves the write.

Before:

```rust
    for _ in 0..2 {
        // SAFETY: `AXValueCreate` copies out of the pointer it is given, and both
        // values live for the call. Each returned value is +1 and released here.
        // `window` is a live element; setting an attribute takes no ownership.
        #[expect(unsafe_code)]
        unsafe {
            let position = AXValueCreate(kAXValueTypeCGPoint, (&raw const origin).cast());
            let extent = AXValueCreate(kAXValueTypeCGSize, (&raw const size).cast());
            AXUIElementSetAttributeValue(
                window,
                CFString::new(kAXPositionAttribute).as_concrete_TypeRef(),
                position.cast(),
            );
            AXUIElementSetAttributeValue(
                window,
                CFString::new(kAXSizeAttribute).as_concrete_TypeRef(),
                extent.cast(),
            );
            CFRelease(position.cast());
            CFRelease(extent.cast());
        }
    }
```

After:

```rust
    for _ in 0..2 {
        set_attribute::<Position>(window, origin);
        set_attribute::<Size>(window, size);
    }
```

```rust
/// Set one `AXValue` attribute of `element`.
fn set_attribute<A: AxAttribute>(element: AXUIElementRef, value: A::Value) {
    // SAFETY: `AXValueCreate` copies out of the pointer it is given, which lives for the
    // call, and returns a +1 reference `Owned` takes responsibility for.
    #[expect(unsafe_code)]
    let Some(boxed) = (unsafe { Owned::new(AXValueCreate(A::KIND, (&raw const value).cast()).cast()) })
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
```

The loop stays. Some apps clamp a move against their current size, so the first position lands short of where it was asked to go and the second lands true.

# Change 4: the frame sink

Observation owns its own state, and it stays on the main thread. The table of elements comes into existence when [`watch`] is called and dies with the [`Watcher`] it returns, the way `freddie_app_nav::watch` already works.\n\nA placement is asked for from the effect loop's thread and performed from the main thread, so nothing is shared and nothing is locked: the sink sends, the main thread looks up, and the `AXUIElement` writes go to a thread of their own.

Until Change 5 registers the observers the table stays empty, and the daemon does not call the sink yet.

```rust
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// A retained `AXUIElement` for one window.
struct Element(Owned);

impl Element {
    /// The element, for the calls that take one. Borrowed, not owned: the release stays
    /// with the [`Owned`] inside.
    fn as_ref(&self) -> AXUIElementRef {
        self.0.0.cast_mut().cast()
    }
}

// SAFETY: an `AXUIElementRef` may be used from any thread. Only `Send` is claimed: an
// `Element` is handed to the thread that performs one placement and is not shared with it,
// so nothing reaches the same one from two threads at once.
#[expect(unsafe_code)]
unsafe impl Send for Element {}

/// The handle a placement is asked for through.
///
/// It does not perform one. It says which window is going where, and the main thread,
/// which owns the table of elements, does the rest. Cheap to clone.
#[derive(Clone)]
pub struct WindowSink {
    placements: Sender<WindowFrame>,
    /// Wakes the main loop so [`Watcher::drain`] runs now rather than up to a slice
    /// later. See the note below on what this has to be.
    waker: Waker,
}

impl WindowSink {
    /// Ask for one window to be moved and resized.
    ///
    /// Fire-and-forget, like `freddie_app_nav::foreground`: it returns as soon as the
    /// request is queued. The frame is the caller's, already worked out; this does not
    /// consult the screen, the frontmost app, or anything else.
    ///
    /// # Errors
    ///
    /// [`WindowError::NotWatching`] if the [`Watcher`] has been dropped, which is the only
    /// thing that can be known here. A window that has closed since is reported by
    /// [`Watcher::drain`], which is where the table is.
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        self.placements
            .send(target)
            .map_err(|_| WindowError::NotWatching)?;
        self.waker.wake();
        Ok(())
    }
}
```

The main thread performs what was asked for, one window at a time:

```rust
impl Watcher {
    /// Perform every placement asked for since the last call. Main thread only, and the
    /// caller is `freddie_main_loop`'s `on_wake`.
    ///
    /// Each one is handed to a thread of its own: the lookup is here, where the table is,
    /// and the `AXUIElement` writes are there, because they cost tens of milliseconds and
    /// the run loop cannot afford them.
    pub fn drain(&self) {
        for target in self.placements.try_iter() {
            let Some(element) = self.observed.elements.borrow().get(&target.window).cloned() else {
                tracing::debug!(window = ?target.window, "no such window to place");
                continue;
            };
            std::thread::spawn(move || {
                set_frame(element.as_ref(), target.frame);
                tracing::debug!(?target, "set a window's frame");
            });
        }
    }
}
```

The new error variants:

```rust
    /// The [`Watcher`] has been dropped, so nothing is being observed at all.
    NotWatching,
```

```rust
            Self::NotWatching => "not watching windows",
```

# Change 5: the observer

One `AXObserver` per app, all of them owned by the watcher, and the events they produce.

```rust
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
```

```rust
/// Report every window change to `on_change`, and return the watcher that owns the
/// observation.
///
/// Observes every running app, and every app that launches while the returned [`Watcher`]
/// is alive. Registering is cheap and takes no thread: each `AXObserver` contributes a run
/// loop source to the main run loop, which `freddie_main_loop` is what gets you into.
///
/// `on_change` runs on the main thread, serialized with every other main-thread callback,
/// so it must hand its work elsewhere and return. Sending on a channel is the intended
/// body, which is what the daemon does.
///
/// The [`Snapshot`] comes back with the watcher rather than from a second call, so no
/// caller can let a report land between reading the starting state and using it. A
/// `Moved` arriving before a snapshot taken after it would overwrite the newer frame with
/// the older one.
///
/// Dropping the [`Watcher`] releases every observer and stops the reports. A
/// [`WindowSink`] taken from it goes on existing and answers
/// [`WindowError::NotWatching`].
pub fn watch(on_change: impl Fn(WindowChange) + Send + Sync + 'static) -> (Watcher, Snapshot) {
    ...
}

/// The live observation. Dropping it stops everything: `apps` goes, which releases every
/// `AXObserver` and removes its run loop source, and the placement receiver goes, which is
/// how a [`WindowSink`] learns it is over. No `Drop` impl needed.
///
/// `!Send`, like `freddie_menu_bar`'s `MenuBar`: it owns main-thread-only state and stays
/// on the thread that built it.
pub struct Watcher {
    /// The `NSWorkspace` observation that keeps `apps` current as apps launch and quit.
    /// Held for its `Drop`, and declared first so it stops before the map it writes into
    /// is torn down: fields drop in declaration order.
    _launches: LaunchWatcher,
    observed: Rc<Observed>,
    /// Placements asked for by a [`WindowSink`], performed by [`drain`](Self::drain).
    /// Dropping the `Watcher` drops this, which is what makes a live sink answer
    /// [`WindowError::NotWatching`].
    placements: Receiver<WindowFrame>,
    /// Kept so [`sink`](Self::sink) can hand out another sender.
    placements_tx: Sender<WindowFrame>,
}

/// Everything observation owns.
///
/// Main thread only, so nothing here is locked: `watch`, the launch and terminate
/// callbacks, every `AXObserver` notification, and [`Watcher::drain`] all run there.
struct Observed {
    /// Every window that can be addressed, and the element to address it through.
    ///
    /// `Arc<Element>`, so [`Watcher::drain`] can hand one to the thread performing a
    /// placement without keeping the map borrowed for the length of the call.
    elements: RefCell<HashMap<WindowId, Arc<Element>>>,
    /// One entry per observed app. Held here rather than on the [`Watcher`] because the
    /// launch and terminate callbacks are `'static` closures that cannot borrow it.
    apps: RefCell<HashMap<pid_t, AppObserver>>,
    on_change: Box<dyn Fn(WindowChange)>,
}

impl Watcher {
    /// A handle to ask for placements through. Cheap to clone, `Send`, and safe to keep
    /// past the watcher, which it answers [`WindowError::NotWatching`] from.
    ///
    /// The `Waker` comes from `freddie_main_loop::main_loop`, so `watch` takes one.
    #[must_use]
    pub fn sink(&self) -> WindowSink {
        WindowSink {
            placements: self.placements_tx.clone(),
            waker: self.waker.clone(),
        }
    }
}

/// One app's observer, and the `refcon` its callbacks reach the observation through.
struct AppObserver {
    observer: AXObserverRef,
    /// The `refcon` every notification for this app carries. Boxed so its address is
    /// stable, and owned here so it is freed exactly when the observer naming it is.
    registration: Box<Registration>,
}

/// What a notification callback needs: the observer to register a new window on, and the
/// observation to report into. A C callback has this instead of a closure.
///
/// `observer` is held rather than a pid, so a window created later is registered without
/// going back through `apps`; nothing in the callback path touches that map.
///
/// [`Weak`](std::rc::Weak), not [`Rc`]: [`Observed`] owns `apps`, an [`AppObserver`] owns
/// its registration, so a strong reference here would be a cycle that never frees.
struct Registration {
    observer: AXObserverRef,
    observed: std::rc::Weak<Observed>,
}
```

The pieces behind `watch`:

- `observe_app(&Rc<Observed>, pid)` calls `AXObserverCreate`, builds the `Registration` holding that observer and a `Weak` back, adds `kAXFocusedWindowChangedNotification` and `kAXWindowCreatedNotification` on the app element with that `refcon`, adds the source to `CFRunLoop::get_main()` under `kCFRunLoopDefaultMode`, inserts the `AppObserver` into `apps`, then walks `kAXWindowsAttribute` once and calls `observe_window` for each.
- `observe_window(&Observed, observer, element)` reads the id, inserts a retained `Element` into `elements`, adds `kAXWindowMovedNotification`, `kAXWindowResizedNotification`, and `kAXUIElementDestroyedNotification` on the window element, and reports `Opened`.
- The app set is kept current from `NSWorkspace`'s `didLaunchApplication` and `didTerminateApplication`, whose closures capture an `rc::Weak<Observed>` for the same reason a `Registration` does, seeded from `runningApplications`. On terminate, the app's `AppObserver` is removed from `apps` and every `elements` entry for its windows is removed, each reported as `Closed`.
- An app that refuses Accessibility, or has not finished launching, fails `AXObserverCreate`. That is logged at `debug` and the app is skipped: its windows are never reported and cannot be addressed, and every other app goes on being observed.

The C callback dispatches on the notification name:

```rust
/// The one `AXObserver` callback. `refcon` is the [`Registration`] the app's
/// [`AppObserver`] owns, which is how a C callback reaches the observation without a
/// global.
///
/// Runs on the main thread, since that is the run loop the sources were added to.
unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    // SAFETY: `refcon` is the `Box<Registration>` this app's `AppObserver` still owns. The
    // observer is released before the box is dropped, so no notification can arrive after
    // the pointer goes stale.
    #[expect(unsafe_code)]
    let registration = unsafe { &*refcon.cast::<Registration>() };

    // The watcher is gone, so there is nothing to report into.
    let Some(observed) = registration.observed.upgrade() else {
        return;
    };

    // SAFETY: `notification` is a live string owned by the caller for this call.
    #[expect(unsafe_code)]
    let name = unsafe { CFString::wrap_under_get_rule(notification) }.to_string();

    match name.as_str() {
        kAXWindowCreatedNotification => observe_window(&observed, registration.observer, element),
        kAXWindowMovedNotification | kAXWindowResizedNotification => {
            if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
                let moved = WindowFrame { window, frame };
                observed.report(if name == kAXWindowMovedNotification {
                    WindowChange::Moved(moved)
                } else {
                    WindowChange::Resized(moved)
                });
            }
        }
        kAXUIElementDestroyedNotification => {
            if let Some(window) = window_id(element) {
                observed.forget(window);
                observed.report(WindowChange::Closed(window));
            }
        }
        kAXFocusedWindowChangedNotification => {
            observed.report(WindowChange::Focused(window_id(element)));
        }
        _ => {}
    }
}
```

`kAXUIElementDestroyedNotification` fires for elements that are not windows, and for a destroyed window the id may already be unreadable. Both come out as no `WindowId` and no report, which is why the app-terminated path also forgets an app's windows: an app quitting is the reliable end of its windows.

A frame that cannot be read produces no report either, which is the same shape and a different consequence. `window_frame` returns `None` when an app is unresponsive or the window is on its way out, and the model keeps whatever frame it last heard about. A stale frame means the next placement picks the monitor the window used to be on, and a restore goes back to a frame measured before the drift. The next move or resize resyncs it, and nothing accumulates. Retrying the read inside the callback is the wrong fix: it runs on the main thread, and an app that will not answer is exactly the one that would stall it.

The screen-change observer that `init` registers belongs to the watcher too: the registration moves into `watch`, and the callback reports `Screens` as well as refreshing `MONITORS`.

`MONITORS` outlives this doc and not the next one. It is a cache of main-thread-only `NSScreen` data for `place`, which runs off the main thread, and `place` is the only thing that reads it. `refactors/pending/placement-in-the-model.md` deletes `place` and puts `screens` in the model, at which point the static has no reader and goes with it, leaving `read_monitors` as a plain function the observer and [`watch`] call.

# Change 6: the seed

The model starts empty and the observer only reports changes, so nothing would tell it about the windows that were already open. The [`Snapshot`] `watch` returns is what does, and it is the analogue of `freddie_app_nav::frontmost`: the starting state, read once.

```rust
/// Every window open right now, which one is focused, and the monitors.
///
/// For seeding a consumer's initial state before [`watch`] starts reporting changes.
/// Polling it is not what it is for: [`watch`] is how you learn about changes.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    pub windows: Vec<WindowFrame>,
    pub focused: Option<WindowId>,
    pub screens: Vec<Monitor>,
}

```

Built inside [`watch`], after the observers are registered and before it returns, so there is no moment when a caller holds a watcher and not a snapshot. It reads `observed.elements` for the windows, which registering just filled, the frontmost app's `kAXFocusedWindowAttribute` for the focus, and `read_monitors` for the screens.

That focus read is the one place this crate asks the OS a question outside a callback. It is the starting value the observer cannot report, because the observer reports changes and none has happened yet.

# Change 7: the model holds the windows

Mercury gains the source, in `crates/mercury/src/sources.rs`, beside `Foregrounded` and `Tabbed`:

```rust
/// The trigger for every window change. One binding at the root answers all of them, the
/// way `Foregrounded` does for app activation.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Windowed;

/// A window change, as the window source reports it.
#[derive(Debug)]
pub struct WindowEvent {
    pub change: WindowChange,
}

impl EventTrigger for Windowed {
    ...
}
```

`MercuryTrigger` gains `Windowed(Windowed)` and `MercuryEvent` gains `Window(WindowEvent)`, each one line beside the existing variants.

The state the root gains:

```rust
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    /// Every window mercury knows about, and the monitors they sit on. See [`Windows`].
    pub windows: Windows,
    ...
}

/// What mercury knows about the windows on screen.
///
/// Filled entirely by the window source: a snapshot at startup and a change per event
/// after it. Handlers read it and never read the OS, so what a placement computes is a
/// function of state and event like everything else.
#[derive(Debug, Default)]
pub struct Windows {
    /// Every open window and where it is.
    frames: HashMap<WindowId, Frame>,
    /// The focused window, `None` when nothing is focused or its id is unreadable.
    focused: Option<WindowId>,
    /// The monitors, in the order the source reported them.
    screens: Vec<Monitor>,
}

impl Windows {
    /// The focused window and its frame, which is what every placement starts from.
    #[must_use]
    pub fn focused(&self) -> Option<WindowFrame> {
        let window = self.focused?;
        Some(WindowFrame {
            window,
            frame: *self.frames.get(&window)?,
        })
    }

    /// The monitor a frame's top-left corner is on, or the first monitor if it is on
    /// none. `None` only before the first `Screens` report.
    #[must_use]
    pub fn monitor_for(&self, frame: Frame) -> Option<Monitor> {
        self.screens
            .iter()
            .find(|m| m.full.contains(frame.x, frame.y))
            .or_else(|| self.screens.first())
            .copied()
    }

    /// Apply one reported change.
    pub(crate) fn record(&mut self, change: &WindowChange) { ... }
}
```

The handler, in a new `crates/mercury/src/handlers/window.rs`:

```rust
/// The windows changed: record it at the root.
///
/// Nothing else happens on a window event. Placements read [`Windows`] when a key asks
/// for one; the source's job is only to keep it true.
pub(crate) fn record_windows(
    ev: &WindowEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.windows.record(&ev.change);
    Vec::new()
}
```

The root binding, beside `Foregrounded`:

```rust
    Windowed => record_windows,
```

The daemon builds the watcher on the main thread, between the event channel and the worker spawn, because a `Watcher` is `!Send` and `drain` runs in the main loop:

```rust
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();

    // Window observation, here rather than in `serve`: the `Watcher` is `!Send`, and
    // `drain` runs on this thread. The worker gets the snapshot as data and a `WindowSink`
    // to ask for placements through.
    let (windows, snapshot) = freddie_windows::watch({
        let event_tx = event_tx.clone();
        move |change| {
            let _ = event_tx.send(window(change));
        }
    });
    let window_sink = windows.sink();
```

`snapshot` and `window_sink` go into the `Boot` struct from `refactors/pending/seed-at-construction.md`, which is what the worker already takes:

```rust
struct Boot {
    front_app: App,
    /// Every window open when the watcher was installed, which one was focused, and the
    /// screens. The observer reports changes, and at boot nothing has changed yet.
    windows: Snapshot,
    /// The handle placements are asked for through.
    window_sink: WindowSink,
}
```

`Mercury::new` takes the snapshot alongside the front app and starts with `Windows` already filled.

```rust
    main_loop.run(|| {
        windows.drain();
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
    });
```

`drain` is what performs a placement, so nothing moves without it.

## Waking the main loop

Without a wake, a placement waits for `freddie_main_loop`'s `SLICE` to expire, because `on_wake` runs after `nextEventMatchingMask_untilDate_inMode_dequeue` returns. That bounds the delay at 100ms rather than leaving it open-ended, but 100ms between the key and the window moving is visible, and the menu-bar title has the same lag today for the same reason.

`CFRunLoop::wake_up` is not established to fix it. It wakes the loop from sleep, but `nextEventMatchingMask` returns when an event is available or its deadline passes, and a bare wake makes neither true. What reliably interrupts that call is posting an application-defined `NSEvent` with `postEvent_atStart`, which makes an event available.

So `Waker` belongs in `freddie_main_loop`, beside the loop it wakes, and `main_loop()` hands one back the way it hands back a `Stopper`:

```rust
/// Wakes the main loop so its `on_wake` runs now rather than when the current slice
/// expires. `Send`, so a worker can hold one.
pub struct Waker { ... }

impl Waker {
    pub fn wake(&self) { ... }
}
```

Both users want it: `WindowSink` holds one, and the title channel should use it rather than waiting out a slice.

One case it does not cover. While a menu is open, AppKit runs a modal tracking loop and the outer pump does not iterate, so `on_wake` does not run until tracking ends whatever is posted. A placement asked for with the status-item menu open is delayed until the menu closes. Acceptable: the menu's only item is Quit.

Nothing can arrive out of order across that seam. A notification is delivered by running the run loop, and the run loop does not run until `main_loop.run`, which is after the worker already holds the snapshot. So the snapshot is the state before any reported change, and every change reported after it is genuinely later.

This is the rule from `refactors/pending/seed-at-construction.md`: reading the OS is what happens before `main_loop.run`, and once it is turning, every fact arrives as an event.

`freddie_windows::init` is called just above this today. It folds into `watch`, per `refactors/pending/placement-in-the-model.md`.

Tests in `crates/mercury/tests/transitions.rs`, over `Windows::record`:

```rust
#[test]
fn an_opened_window_is_recorded_with_its_frame() { ... }

#[test]
fn a_move_replaces_the_frame_and_keeps_the_focus() { ... }

#[test]
fn a_closed_window_leaves_no_frame_and_no_focus() { ... }

/// A focus report for a window no report ever mentioned leaves `focused()` empty rather
/// than naming a window with no frame.
#[test]
fn focus_on_an_unknown_window_yields_nothing_focused() { ... }

/// The seed replaces everything, so a snapshot after a reconnect cannot leave a window
/// behind that closed while nothing was listening.
#[test]
fn a_snapshot_replaces_what_was_there() { ... }
```
