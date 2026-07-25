# a closing window reports itself

`WindowChange::Closed` is not reported when a window closes. The destroy branch asks the destroyed element for its id, and a destroyed element does not answer:

```rust
} else if name == kAXUIElementDestroyedNotification {
    if let Some(window) = window_id(element) {
        state.forget(window);
        state.report(WindowChange::Closed(window));
    }
}
```

`window_id` calls `_AXUIElementGetWindow` on an element the app has already torn down. It returns non-zero, `window_id` gives `None`, and the window is neither forgotten nor reported.

`Elements` only shrinks when an observed app terminates, so it holds one retained `AXUIElement` per window ever opened, and `WindowSink::set_frame` writes through a dead element instead of answering `WindowError::UnknownWindow`.

`Windows::open` in mercury (`crates/mercury/src/state/mod.rs`) only shrinks on `Closed`, so it holds one `WindowState` per window ever opened, and `focused` keeps naming a window that is gone. `Windows`'s `Debug` prints its name and nothing else, so the log does not show the growth.

`forget_app` is the only path that reports `Closed` today, and it is wrong: it sweeps the whole element table for entries whose id no longer reads, so one app quitting reports every window earlier apps left behind.

Separately, `observe_app` abandons the observer it just created when the app element cannot be made:

```rust
let status = unsafe { AXObserverCreate(pid.0, on_notification, &raw mut observer) };
// ...
let app = unsafe { AXUIElementCreateApplication(pid.0) };
let Some(app) = Owned::new(app.cast()) else {
    return;                 // `observer` is +1, is not in `state.apps`, and nothing releases it
};
```

Nothing has taken ownership of `observer` at that point, so no `AppObserver::drop` ever runs for it.

## Shape after

The `refcon` a notification carries is the only thing that still names a window once the window is gone, so the window's id goes in it. One `Registration` per app element as now, plus one per window, and the destroy branch reads the id out of its own registration rather than out of the element.

```rust
/// What one registration's notifications are about.
///
/// `kAXUIElementDestroyed` arrives for an element the app has already torn down, and
/// `_AXUIElementGetWindow` on one of those answers with nothing, so the id a destroyed window
/// is reported under is the one recorded here when it was still live.
#[derive(Clone, Copy)]
enum Subject {
    /// The app element: focus changes, and windows being created.
    App,
    /// One window: its moves, its resizes, and its destruction.
    Window(WindowId),
}

/// What a notification callback needs: the observer to register a new window on, the pid of
/// the app it is for, what the registration is about, and the state to report into. A C
/// callback has this instead of a closure.
///
/// `observer` is held so a window created later is registered without going back through
/// `apps` for it. `pid` is what a focus-changed notification is gated on, so only the frontmost
/// app's focused window is reported and a background app changing its own focus is ignored, and
/// it is also which app's `AppObserver` a new window's registration belongs to.
///
/// [`Weak`](std::rc::Weak), not [`Rc`]: [`WatcherState`] owns `apps`, an [`AppObserver`] owns its
/// registrations, so a strong reference here would be a cycle that never frees.
struct Registration {
    observer: AXObserverRef,
    pid: Pid,
    subject: Subject,
    state: std::rc::Weak<WatcherState>,
}

/// One app's observer, and the `refcon`s its callbacks reach the [`Watcher`]'s state through.
struct AppObserver {
    observer: AXObserverRef,
    /// The app element's `refcon`. Boxed so its address is stable, and owned here so it is
    /// freed exactly when the observer naming it is.
    _app: Box<Registration>,
    /// One `refcon` per window this observer was given, in the order they arrived. Each is
    /// boxed so its heap address is stable under `Vec` reallocation; the framework keeps that
    /// address as the notification `refcon`. Kept for the life of the observer, not until its
    /// window closes, so the framework can never hand back a freed address. All of them go when
    /// the app quits.
    #[expect(clippy::vec_box)]
    windows: Vec<Box<Registration>>,
}
```

A `Registration` is 32 bytes and a `Vec` slot is 8, so a window costs 40 bytes for as long as its app runs. The three per-window notification registrations stay with the framework until the observer is released, which is what they do now.

`Drop` is unchanged and still correct for both fields: the body removes the run loop source and releases the observer, and `_app` and `windows` drop after the body, so no box is freed while a source that could name it is still in the loop.

`on_notification` copies every `Copy` field out of the `Registration` and ends that borrow before any work that touches `apps`. `observe_window` pushes into `AppObserver::windows`, which is a `borrow_mut` on the same `AppObserver` the app `refcon` points into; holding `&Registration` across that would alias.

## Change 0: `observe_app` creates the app element before the observer

File: `crates/freddie_windows/src/lib.rs`. Independent of Change 1.

The two `Create` calls swap order. Nothing fallible then sits between `AXObserverCreate` and the `state.apps` insert that takes ownership of what it returned.

### Before

```rust
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
        pid,
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
```

### After

```rust
fn observe_app(state: &Rc<WatcherState>, ObservableApp(pid): ObservableApp) {
    if state.apps.borrow().contains_key(&pid) {
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
    let status = unsafe { AXObserverCreate(pid.0, on_notification, &raw mut observer) };
    if status != 0 || observer.is_null() {
        tracing::debug!(?pid, status, "could not observe an app");
        return;
    }

    let registration = Box::new(Registration {
        observer,
        pid,
        state: Rc::downgrade(state),
    });
    let refcon = std::ptr::from_ref(registration.as_ref()).cast_mut().cast();
```

The rest of the function is untouched by this change: `add_notification` twice on `app_element`, `add_source`, the `apps` insert, then the `app_windows` loop.

## Change 1: the destroy notification carries its window id

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 0 only for the ordering it leaves behind.

### `Subject`, `Registration`, `AppObserver`

`Subject` is new, as written in "Shape after". `Registration` gains `subject: Subject`. `AppObserver` renames `_registration` to `_app` and gains `windows: Vec<Box<Registration>>`.

`AppObserver` before:

```rust
struct AppObserver {
    observer: AXObserverRef,
    /// The `refcon` every notification for this app carries. Boxed so its address is
    /// stable, and owned here so it is freed exactly when the observer naming it is.
    _registration: Box<Registration>,
}
```

`AppObserver` after: as written in "Shape after".

### `WatcherState::forget` says whether it removed anything

So that a window is reported closed once, whether its own notification or its app quitting got there first.

Before:

```rust
    /// Stop being able to address `window`.
    fn forget(&self, window: WindowId) {
        if let Ok(mut table) = self.elements.0.lock() {
            table.remove(&window);
        }
    }
```

After:

```rust
    /// Stop being able to address `window`. Whether there was an entry to remove, which is
    /// whether this is the report that closes it: a window's own `AXUIElementDestroyed` and
    /// its app terminating both arrive, in either order, and only the first of them reports.
    fn forget(&self, window: WindowId) -> bool {
        self.elements
            .0
            .lock()
            .is_ok_and(|mut table| table.remove(&window).is_some())
    }
```

### `on_notification`

Copies every `Copy` field out of the `Registration` and ends that borrow before any branch runs. The created branch then hands `observe_window` the pid instead of the app's `refcon`, because the window gets a `refcon` of its own. The destroyed branch reads the id it was registered under.

Before:

```rust
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
        // Only the frontmost app's focused window is what a placement aims at; a background
        // app changing its own focus is not the window the user is looking at.
        if is_frontmost(registration.pid) {
            state.report(WindowChange::Focused(window_id(element)));
        }
    }
}
```

After:

```rust
#[expect(unsafe_code)]
unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    // Copy every `Copy` field out and end the `&Registration` borrow before any branch
    // runs. `observe_window` does `apps.borrow_mut()` and pushes into the same
    // `AppObserver` the app `refcon` points into; holding that reference across the push
    // would alias.
    let (state, observer, pid, subject) = {
        // SAFETY: `refcon` is a `Box<Registration>` this app's `AppObserver` still owns
        // (in `_app` or `windows`). The observer's source is removed before those boxes are
        // dropped, so no notification can arrive after the pointer goes stale.
        let registration = unsafe { &*refcon.cast::<Registration>() };
        let Some(state) = registration.state.upgrade() else {
            return;
        };
        (
            state,
            registration.observer,
            registration.pid,
            registration.subject,
        )
    };

    // SAFETY: `notification` is a live string owned by the caller for this call.
    let name = unsafe { CFString::wrap_under_get_rule(notification) }.to_string();

    let name = name.as_str();
    // Comparisons rather than match arms: these constants are lowercase, and a lowercase
    // path in a pattern binds rather than matches the moment it stops resolving.
    if name == kAXWindowCreatedNotification {
        observe_window(&state, observer, pid, element);
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
        // The element is gone and cannot be asked what it was, so the id comes from this
        // registration, which is the window's own.
        if let Subject::Window(window) = subject
            && state.forget(window)
        {
            state.report(WindowChange::Closed(window));
        }
    } else if name == kAXFocusedWindowChangedNotification {
        // Only the frontmost app's focused window is what a placement aims at; a background
        // app changing its own focus is not the window the user is looking at.
        if is_frontmost(pid) {
            state.report(WindowChange::Focused(window_id(element)));
        }
    }
}
```

`refcon` is not read after the block that produces the copied fields. `Subject` is `Copy`, so the destroy branch does not need the registration to stay borrowed.

### `observe_window`

Takes `pid` rather than a `refcon`, and makes the `refcon` its window's notifications carry.

Before:

```rust
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
```

After:

```rust
/// Record a window, subscribe to what it does, and keep the `refcon` those notifications
/// carry. Nothing is announced here.
///
/// The setup pass calls this alone: every window it finds is already in the `Snapshot` `watch`
/// returns, so reporting `Opened` for it would be a redundant replay of the seed. A window that
/// opens later goes through here too, and `on_notification` then calls `report_open`; see its
/// call site.
///
/// The `refcon` is the window's own [`Registration`], not its app's, which is how
/// `kAXUIElementDestroyed` names the window it is about after the element is gone. The app's
/// [`AppObserver`] takes the box before any notification names it and holds it until its
/// observer is released, so the address the framework keeps stays valid for as long as it
/// could be handed back.
fn observe_window(
    state: &Rc<WatcherState>,
    observer: AXObserverRef,
    pid: Pid,
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

    let registration = Box::new(Registration {
        observer,
        pid,
        subject: Subject::Window(window),
        state: Rc::downgrade(state),
    });
    // The box's heap address, which it keeps when it moves into `windows` below.
    let refcon = std::ptr::from_ref(registration.as_ref()).cast_mut().cast();
    {
        let mut apps = state.apps.borrow_mut();
        let Some(app) = apps.get_mut(&pid) else {
            return;
        };
        app.windows.push(registration);
    }

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
```

The order is what makes the `refcon` sound. The box belongs to the app's `AppObserver` before the first `add_notification` names it, and the one path that does not get it there returns before any notification is added.

`state` is `&Rc<WatcherState>` now, for the `Rc::downgrade`. Both call sites already hold one: `on_notification` upgraded a `Weak` into `state`, and `observe_app` takes `&Rc<WatcherState>`.

### `observe_app`

Three lines change, on top of Change 0's reordering. `registration` names its subject, the `AppObserver` is built with an empty `windows`, and the setup loop passes `pid`.

Before:

```rust
    let registration = Box::new(Registration {
        observer,
        pid,
        state: Rc::downgrade(state),
    });
```

After:

```rust
    let registration = Box::new(Registration {
        observer,
        pid,
        subject: Subject::App,
        state: Rc::downgrade(state),
    });
```

Before:

```rust
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
```

After:

```rust
    state.apps.borrow_mut().insert(
        pid,
        AppObserver {
            observer,
            _app: registration,
            windows: Vec::new(),
        },
    );

    // After the insert: `observe_window` puts each window's `refcon` in the entry this made.
    for window in app_windows(app_element) {
        observe_window(state, observer, pid, window.raw());
    }
```

`refcon` is now used only by the two `add_notification` calls on `app_element`.

### `forget_app`

Reports the quitting app's own windows, named by its own registrations, so nothing asks a dead element for an id and no other app's windows are swept up in it. Window ids are collected and the observer is dropped before any `Closed` is reported, so its run loop source is gone before `on_change` runs and cannot deliver a late notification into a map entry that is already gone.

Before:

```rust
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
```

After:

```rust
/// Stop watching an app, reporting every window it took with it.
///
/// The windows are the ones this app's own registrations name, so a window another app still
/// has open is not reported closed, and no dead element is asked for an id. A window whose own
/// `AXUIElementDestroyed` already arrived was forgotten then, and `forget` returning false is
/// what keeps it from being reported twice.
///
/// The observer is dropped before any `Closed` is reported: that removes its run loop source
/// so a late notification cannot run against an `apps` map that no longer holds it.
fn forget_app(state: &WatcherState, pid: Pid) {
    let Some(observer) = state.apps.borrow_mut().remove(&pid) else {
        return;
    };
    let windows: Vec<WindowId> = observer
        .windows
        .iter()
        .filter_map(|registration| match registration.subject {
            Subject::Window(window) => Some(window),
            Subject::App => None,
        })
        .collect();
    drop(observer);
    for window in windows {
        if state.forget(window) {
            state.report(WindowChange::Closed(window));
        }
    }
}
```

`window_id` loses this call site and keeps the three it has in `on_notification`, `observe_window` and `report_open`, all on live elements.

A window whose `kAXUIElementDestroyed` never arrives because `add_notification` refused it is still reported when its app quits, and held until then.

## Call sites

`freddie_windows`'s public surface does not change: `watch`, `Watcher`, `WindowSink`, `Snapshot` and `WindowChange` all keep their shapes, and every function this touches is private to the crate.

`crates/mercury/src/state/mod.rs` already handles `WindowChange::Closed` by removing the window from `open` and clearing `focused` if it named it, so mercury needs no change. It starts receiving the event, which is the point: `Windows::open` tracks the windows that exist rather than every window the run ever saw, and a placement aimed at a closed window gets `WindowError::UnknownWindow` from `WindowSink::set_frame` instead of writing a frame through a retained dead element.

## Verification

```
cargo test -p freddie_windows
```

The crate's tests cover `Frame` geometry and are untouched. Observation needs a window server, a second app, and the Accessibility permission, so what the change does is checked by hand against a running daemon.

After `mercury restart`:

1. One window. Open a window in any app, note its id in the `Opened` record, close it. A `Closed` for that same id arrives within milliseconds, on its own rather than in a burst.
2. One app. Quit an app holding several windows. One `Closed` per window it had, and none for any window another app still has open.
3. Over a session. `Closed` tracks `Opened` instead of sitting at zero:

```
PID=$(pgrep -f 'mercury daemon')
LOG=~/Library/Logs/mercury/mercury.log
grep "\"pid\":$PID" $LOG | grep -c 'change: Opened'
grep "\"pid\":$PID" $LOG | grep -c 'change: Closed'
```

The two counts differ by the windows that are still open.

## Ordered commits

1. Change 0: `observe_app` creates the app element before the observer.
2. Change 1: `Subject`; per-window `Registration` in `AppObserver::windows`; `forget` returns whether it removed; `on_notification` ends the registration borrow before any branch; the destroy branch and `forget_app` report from registrations.
