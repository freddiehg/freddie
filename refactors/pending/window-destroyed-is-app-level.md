# the destroyed notification is app-level

`kAXUIElementDestroyedNotification` is registered on the window element, where it never fires. `AXObserverAddNotification` returns success for it, so nothing reports a problem, and no destroy notification is ever delivered.

Measured against Finder, opening a window and closing it, with everything else held the same:

```
--- destroy registered on the window element (what the code does now) ---
    AXWindowCreated on app: status=0
    created: element=0x9eedb4090 id=4231
      registered destroy ON THE WINDOW: status=0
    => destroy delivered: False

--- destroy registered on the app element ---
    AXWindowCreated on app: status=0
    AXUIElementDestroyed on app: status=0
    DESTROY fired: element=0x9eedb41b0 getwindow_status=-25201 id=0
      same pointer as created? False   CFEqual to retained? True   hash match? True
    => destroy delivered: True
```

So `WindowChange::Closed` is still never reported, and `Elements` still holds one retained `AXUIElement` per window ever opened. Forty Finder windows opened and closed give `Opened=+40 Closed=+0` and about 3 allocations per window that are never freed. `Windows::open` in mercury grows with it, and `WindowSink::set_frame` still writes through dead elements.

`kAXWindowMoved` and `kAXWindowResized` do fire from a window-element registration; 200 nudges produce 200 `Moved` events. Only the destroyed notification is app-level.

Two things follow, and together they decide the design.

An app-level registration reports every element that app destroys, not only its windows. One window close delivered six destroy notifications, five of them for elements that were never in the table. So the handler cannot assume the element it is handed is a window.

One app-level registration serves every element, so a `refcon` cannot name the window. A per-window `refcon` is only reachable from a per-window registration, and that is the registration that does not fire.

What does work is identity. `CFEqual` and `CFHash` both still answer for a destroyed element and both match the retained one, while the pointer does not: the notification's element was `0x9eedb41b0` against a retained `0x9eedb4180`, and `CFEqual` matched anyway. `_AXUIElementGetWindow` continues to fail with `-25201`, so the id must come from the table rather than from the element.

## Shape after

The destroyed notification moves to the app element, and the window it names is found by identity against the retained elements. `Subject`, the per-window `Registration`, and `AppObserver::windows` all go: with the id coming from the table, every notification can share one `refcon` per app again.

```rust
/// One app's observer, and the `refcon` its callbacks reach the [`Watcher`]'s state through.
struct AppObserver {
    observer: AXObserverRef,
    /// The `refcon` every notification for this app carries. Boxed so its address is
    /// stable, and owned here so it is freed exactly when the observer naming it is.
    _registration: Box<Registration>,
}

/// What a notification callback needs: the observer to register a new window on, the pid of
/// the app it is for, and the state to report into. A C callback has this instead of a closure.
struct Registration {
    observer: AXObserverRef,
    pid: Pid,
    state: std::rc::Weak<WatcherState>,
}
```

`WatcherState` gains the lookup that the destroy branch needs:

```rust
    /// Forget whichever window `element` names, and say which it was.
    ///
    /// By identity rather than by id: `kAXUIElementDestroyed` arrives for an element the app has
    /// already torn down, `_AXUIElementGetWindow` refuses it, and `CFEqual` still matches the
    /// element that was retained when the window opened. `None` when the element was not a window
    /// this was watching, which is most of them: the notification is registered on the app, so it
    /// reports every element the app destroys.
    fn forget_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        let mut table = self.elements.0.lock().ok()?;
        // SAFETY: both are live `AXUIElement`s as far as CoreFoundation is concerned. A destroyed
        // element is still a valid CF object; it is the Accessibility calls on it that fail.
        #[expect(unsafe_code)]
        let found = table
            .iter()
            .find(|(_, held)| unsafe { CFEqual(held.raw().cast(), element.cast()) })
            .map(|(id, _)| *id)?;
        table.remove(&found);
        Some(found)
    }
```

The scan is linear over the table, and it runs once per destroyed element rather than once per event. A window close delivers six, and the table holds a few dozen entries in normal use, so this is tens of comparisons per close.

`CFEqual` comes from `core_foundation::base`, which the crate already imports `CFRelease`, `CFRetain`, `CFTypeRef` and `TCFType` from.

## Change 1: register the destroyed notification on the app element

File: `crates/freddie_windows/src/lib.rs`.

### `observe_app` before

```rust
    for notification in [
        kAXFocusedWindowChangedNotification,
        kAXWindowCreatedNotification,
    ] {
        add_notification(observer, app_element, notification, refcon);
    }
```

### `observe_app` after

```rust
    for notification in [
        kAXFocusedWindowChangedNotification,
        kAXWindowCreatedNotification,
        // On the app element, not on each window: a window-element registration for this one
        // returns success and never fires.
        kAXUIElementDestroyedNotification,
    ] {
        add_notification(observer, app_element, notification, refcon);
    }
```

### `observe_window` before

```rust
    for notification in [
        kAXWindowMovedNotification,
        kAXWindowResizedNotification,
        kAXUIElementDestroyedNotification,
    ] {
        add_notification(observer, element, notification, refcon);
    }
```

### `observe_window` after

```rust
    for notification in [kAXWindowMovedNotification, kAXWindowResizedNotification] {
        add_notification(observer, element, notification, refcon);
    }
```

`observe_window` also loses the per-window `Registration`, so it goes back to taking the app's `refcon` and needs neither `pid` nor `&Rc<WatcherState>`:

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

    for notification in [kAXWindowMovedNotification, kAXWindowResizedNotification] {
        add_notification(observer, element, notification, refcon);
    }

    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(window, Arc::new(Element(owned)));
    }
}
```

### The destroy branch before

```rust
    } else if name == kAXUIElementDestroyedNotification {
        // The element is gone and cannot be asked what it was, so the id comes from this
        // registration, which is the window's own.
        if let Subject::Window(window) = subject
            && state.forget(window)
        {
            state.report(WindowChange::Closed(window));
        }
```

### The destroy branch after

```rust
    } else if name == kAXUIElementDestroyedNotification {
        // Registered on the app, so this reports every element the app destroys. The element
        // cannot be asked for its id, and `CFEqual` still matches the one retained when the
        // window opened, so the table answers instead. `None` for anything that was not a
        // window being watched.
        if let Some(window) = state.forget_element(element) {
            state.report(WindowChange::Closed(window));
        }
```

### `on_notification`'s copied fields

The `Subject` field goes, so the block that copies out of the `Registration` yields three values rather than four. The aliasing reason for that block stands: `observe_window` no longer touches `apps`, but the borrow is ended before dispatch anyway, and the created branch still hands on the `refcon`.

```rust
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
```

### `forget_app`

Unchanged in shape, but it can no longer read window ids out of per-window registrations. It goes back to naming the app's windows by the only thing that still distinguishes them, which is that their elements no longer answer:

```rust
/// Stop watching an app, reporting every window it took with it.
///
/// The observer is dropped before any `Closed` is reported, so its run loop source is gone
/// before the reports run. A window whose own destroyed notification already arrived was
/// forgotten then and is not in the table to report twice.
fn forget_app(state: &WatcherState, pid: Pid) {
    let Some(observer) = state.apps.borrow_mut().remove(&pid) else {
        return;
    };
    drop(observer);
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
        if state.forget(window) {
            state.report(WindowChange::Closed(window));
        }
    }
}
```

With destroy reporting for real, this path is now the fallback rather than the only one: it catches windows whose destroyed notification never arrived, and an app that quits outright takes its windows down without individual notifications.

`WatcherState::forget` keeps returning `bool`, which is what stops a window being reported twice when both paths see it.

## Call sites

None outside `crates/freddie_windows/src/lib.rs`. `WindowChange` and `Snapshot` are unchanged, and mercury already handles `Closed` by removing the window from `open` and clearing `focused`.

## Verification

`cargo test -p freddie_windows` passes; the crate's tests are `Frame` geometry and are untouched.

By hand, after `mercury restart`, because this cannot be unit tested without a window server and the Accessibility grant:

1. Open a window in any app, note the id in the `Opened` record, close it. A `Closed` for that same id arrives within milliseconds.
2. Over a run, `Closed` tracks `Opened` instead of sitting at zero:

```
PID=$(pgrep -f 'target/debug/mercury daemon')
LOG=~/Library/Logs/mercury/mercury.log
grep "\"pid\":$PID" $LOG | grep -c 'change: Opened'
grep "\"pid\":$PID" $LOG | grep -c 'change: Closed'
```

3. The allocation count holds flat across window churn. Forty windows opened and closed currently costs about 120 to 160 allocations that are never freed:

```
vmmap -summary $PID | grep DefaultMallocZone
```

## Ordered commits

1. Change 1: the destroyed notification on the app element, `forget_element` by `CFEqual`, `Subject` and the per-window registrations removed, `forget_app` back to the element scan as a fallback.
