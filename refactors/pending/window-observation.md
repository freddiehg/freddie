# Window observation

`freddie_windows` becomes a source as well as a sink, the shape `freddie_app_nav` already has: [`watch`] reports what windows are doing, [`set_frame`] asks for a change, [`snapshot`] seeds the initial state, and nothing ties a call to a report.

Mercury's model ends up holding the geometry: every window's id and frame, which one is focused, and the monitors. It is filled by events like `foreground` and the Chrome tab URL are, so dispatch reads no OS state and an effect carries everything it needs.

Nothing consumes the table when this doc is done. `Place(Placement)` still works exactly as it does now. `refactors/pending/placement-in-the-model.md` is what makes the placement path use it, and `refactors/pending/window-restore.md` is what adds `r`.

## What is observed

Per app, one `AXObserver` on the application element, created when the app appears and released when it quits. On the application element:

- `kAXFocusedWindowChangedNotification`
- `kAXWindowCreatedNotification`

On each window element, added as the window is seen:

- `kAXWindowMovedNotification` and `kAXWindowResizedNotification`, which are the same event to mercury: the frame changed.
- `kAXUIElementDestroyedNotification`.

An `AXObserver` gives a `CFRunLoopSource`. It is added to the main run loop, which `freddie_main_loop` is already inside, so a callback runs there exactly as `freddie_app_nav`'s does. Observation is per-pid and costs no thread and no poll.

An app that refuses Accessibility, or has not finished launching, fails `AXObserverCreate`. That is logged at `debug` and the app is skipped: its windows are invisible to mercury, which loses restore for them and nothing else.

## Identity

A window's identity is its `CGWindowID`. An `AXUIElementRef` is not it: elements are created per call and two for the same window are different pointers.

`_AXUIElementGetWindow` is the only way across. It is private, exported by HIServices, and has been there since 10.x. A window whose id cannot be read produces no events, so a future removal of the symbol costs window observation and leaves the rest of the crate standing.

The crate keeps the reverse direction too: a table from `WindowId` to the retained element observing it. That is what makes [`set_frame`] a lookup into a table the observer already maintains rather than a walk of every app's `kAXWindowsAttribute`. It is also the only place that mapping exists, so the model and the effects speak `WindowId` alone.

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
/// Read one `AXValue` attribute of `element` into `out`, which names the type to unwrap:
/// a `CGPoint` for `kAXValueTypeCGPoint`, a `CGSize` for `kAXValueTypeCGSize`.
fn ax_value<T: Copy>(
    element: AXUIElementRef,
    attribute: &str,
    kind: AXValueType,
    mut out: T,
) -> Option<T> {
    let attribute = CFString::new(attribute);
    let mut value: *const c_void = std::ptr::null();
    // SAFETY: `element` is live and `attribute` a live string. On success the
    // out-parameter receives a +1 `AXValue`; on failure it is untouched.
    #[expect(unsafe_code)]
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }

    // SAFETY: `value` is a +1 `AXValue` of `kind`, which the caller pairs with `T`;
    // `AXValueGetValue` copies it into `out`. The value is released afterward.
    #[expect(unsafe_code)]
    let got = unsafe {
        let ok = AXValueGetValue(
            value.cast_mut().cast(),
            kind,
            std::ptr::from_mut(&mut out).cast(),
        );
        CFRelease(value);
        ok
    };
    got.then_some(out)
}

/// A window's frame, in Accessibility coordinates, or `None` if either half of it
/// cannot be read.
fn window_frame(window: AXUIElementRef) -> Option<Frame> {
    let origin = ax_value(
        window,
        kAXPositionAttribute,
        kAXValueTypeCGPoint,
        CGPoint::new(0.0, 0.0),
    )?;
    let size = ax_value(
        window,
        kAXSizeAttribute,
        kAXValueTypeCGSize,
        CGSize::new(0.0, 0.0),
    )?;
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

# Change 3: the element table and the frame sink

The table the observer will fill, and the sink that reads it. Until Change 4 nothing writes the table, so `set_frame` finds nothing and the daemon does not call it yet.

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

/// A retained `AXUIElement` for one window, keyed by the id everything else names it by.
///
/// `AXUIElementRef` is a Core Foundation type, so a raw pointer is not `Send`. The
/// elements here are only ever created, used, and released on the main thread, which is
/// where every observer callback and every table write runs. `Element` is what says so.
struct Element(AXUIElementRef);

// SAFETY: the pointer is only dereferenced on the main thread. The table is behind a
// lock so the pointer can be copied out, and copying is all any other thread does before
// handing the work back to main.
#[expect(unsafe_code)]
unsafe impl Send for Element {}
// SAFETY: as above.
#[expect(unsafe_code)]
unsafe impl Sync for Element {}

impl Drop for Element {
    fn drop(&mut self) {
        // SAFETY: every `Element` is built from a +1 reference; this balances it.
        #[expect(unsafe_code)]
        unsafe {
            CFRelease(self.0.cast());
        }
    }
}

/// Every window mercury can address, and the element to address it through. Filled and
/// emptied by the observer callbacks in Change 4.
static ELEMENTS: LazyLock<RwLock<HashMap<WindowId, Element>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Move and resize one window, named by id.
///
/// Immediate, with no animation. Costs single-digit to low tens of milliseconds, so a
/// caller on a latency-sensitive loop should hand this to another thread.
///
/// The frame is the caller's, already computed. This does not consult the screen, the
/// frontmost app, or anything else: it is the sink, and everything it needs is in its
/// arguments.
///
/// # Errors
///
/// [`WindowError::UnknownWindow`] if no window with that id is being observed, which is
/// the case for a window that has closed or that never reported an id.
pub fn set_frame_of(window: WindowId, frame: Frame) -> Result<(), WindowError> {
    let elements = ELEMENTS.read().map_err(|_| WindowError::UnknownWindow)?;
    let element = elements.get(&window).ok_or(WindowError::UnknownWindow)?;
    set_frame(element.0, frame);
    tracing::debug!(?window, ?frame, "set a window's frame");
    Ok(())
}
```

The new error variant:

```rust
    /// Nothing with that id is being observed: the window closed, or it never reported
    /// an id to begin with.
    UnknownWindow,
```

```rust
            Self::UnknownWindow => "no such window",
```

# Change 4: the observer

One `AXObserver` per app, and the events they produce.

```rust
/// What the windows are doing. One variant per thing the observer can tell you.
#[derive(Clone, PartialEq, Debug)]
pub enum WindowChange {
    /// A window appeared, with the frame it appeared at.
    Opened(WindowFrame),
    /// A window's frame changed: moved, resized, or both. The two notifications are one
    /// change, because the frame is what mercury keeps.
    Moved(WindowFrame),
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

`watch` mirrors `freddie_app_nav::watch`: it registers and returns a guard, and the callback runs on the main thread and must return quickly.

```rust
/// Report every window change to `on_change`.
///
/// Observes every running app, and every app that launches while the returned guard is
/// alive. Registering is cheap and takes no thread: each `AXObserver` contributes a run
/// loop source to the main run loop, which `freddie_main_loop` is what gets you into.
///
/// `on_change` runs on the main thread, serialized with every other main-thread callback,
/// so it must hand its work elsewhere and return. Sending on a channel is the intended
/// body, which is what the daemon does.
///
/// Dropping the guard stops the reports and releases every observer.
pub fn watch(on_change: impl Fn(WindowChange) + Send + Sync + 'static) -> Watcher {
    ...
}
```

The pieces behind it:

- A static `OBSERVERS: LazyLock<RwLock<HashMap<pid_t, AppObserver>>>`, one entry per observed app. `AppObserver` owns the `AXObserverRef` and releases it on `Drop`, which also removes its run loop source.
- A static `ON_CHANGE` holding the callback, set by `watch` and cleared by the guard's `Drop`. The C callback has no useful `refcon` route to a Rust closure that can outlive a single app, so the callback is global and the `refcon` carries the pid.
- `observe_app(pid)` calls `AXObserverCreate`, adds `kAXFocusedWindowChangedNotification` and `kAXWindowCreatedNotification` on the app element, adds the source to `CFRunLoop::get_main()` under `kCFRunLoopDefaultMode`, then walks `kAXWindowsAttribute` once and calls `observe_window` for each.
- `observe_window(pid, element)` reads the id, inserts into `ELEMENTS`, adds `kAXWindowMovedNotification`, `kAXWindowResizedNotification`, and `kAXUIElementDestroyedNotification` on the window element, and reports `Opened`.
- The app set is kept current from `NSWorkspace`'s `didLaunchApplication` and `didTerminateApplication` notifications, seeded from `runningApplications`. On terminate, the app's `AppObserver` is dropped and every `ELEMENTS` entry for its windows is removed, each reported as `Closed`.

The C callback dispatches on the notification name:

```rust
/// The one `AXObserver` callback. `refcon` carries the pid whose observer fired, which is
/// what a per-app registration has instead of a closure.
///
/// Runs on the main thread, since that is the run loop the sources were added to.
unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    // SAFETY: `notification` is a live string owned by the caller for this call.
    #[expect(unsafe_code)]
    let name = unsafe { CFString::wrap_under_get_rule(notification) }.to_string();
    let pid = refcon as pid_t;

    match name.as_str() {
        kAXWindowCreatedNotification => observe_window(pid, element),
        kAXWindowMovedNotification | kAXWindowResizedNotification => {
            if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
                report(WindowChange::Moved(WindowFrame { window, frame }));
            }
        }
        kAXUIElementDestroyedNotification => {
            if let Some(window) = window_id(element) {
                forget_window(window);
                report(WindowChange::Closed(window));
            }
        }
        kAXFocusedWindowChangedNotification => report(WindowChange::Focused(window_id(element))),
        _ => {}
    }
}
```

`kAXUIElementDestroyedNotification` fires for elements that are not windows, and for a destroyed window the id may already be unreadable. Both come out as no `WindowId` and no report, which is why `forget_window` also runs from the app-terminated path: an app quitting is the reliable end of its windows.

The screen-change observer `init` already registers reports `Screens` as well as refreshing `MONITORS`:

```rust
    let block = RcBlock::new(|_notif: NonNull<NSNotification>| {
        if let Some(mtm) = MainThreadMarker::new() {
            let monitors = read_monitors(mtm);
            store_monitors(monitors.clone());
            report(WindowChange::Screens(monitors));
            tracing::debug!("re-read monitors after a screen-arrangement change");
        }
    });
```

# Change 5: the seed

The model starts empty and the observer only reports changes, so nothing would tell it about the windows that were already open. `snapshot` is the analogue of `freddie_app_nav::frontmost`: good for seeding, and for nothing else.

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

#[must_use]
pub fn snapshot() -> Snapshot { ... }
```

It reads `ELEMENTS` for the windows, since `watch` filled it while registering, the frontmost app's `kAXFocusedWindowAttribute` for the focus, and `MONITORS` for the screens. So the daemon calls `watch` first and `snapshot` second, and the order is documented on both.

# Change 6: the model holds the geometry

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

The daemon wires the source, beside the `freddie_app_nav::watch` call, and seeds from the snapshot after registering.

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
