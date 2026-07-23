# The boot snapshot is the seed, not a replay

At boot `freddie_windows::watch` reports one `Opened` per window that was already open, on top of returning a `Snapshot` that already lists every one of them. The daemon builds the model from the snapshot (`Windows::from_snapshot`) and then dispatches the `Opened` events into it, each matching a window the state already holds and producing `effects=[]`. Those events carry nothing the snapshot did not: they are the setup pass announcing pre-existing windows through the same path new windows use.

After this change, `watch`'s setup pass records each existing window silently, and only a window that opens after the observers are installed reports `Opened`. The snapshot is the sole seed. In steady boot the model dispatches zero window events instead of one per open window.

## Why

`observe_window` (`crates/freddie_windows/src/lib.rs`) does two jobs in one function: it registers the per-window notifications and inserts the element into `state.elements` (discovery), and it calls `state.report(WindowChange::Opened(..))` (announce). It is called from two sites:

- `observe_app`, once per window an app already has, during `watch`'s setup pass (`for window in app_windows(app_element)`).
- `on_notification`, on `kAXWindowCreatedNotification`, once per window that opens later.

`watch` then reads the `Snapshot` off the same `state.elements` table it just populated. So every window present at install time is delivered twice: once in the snapshot that constructs `Windows`, and once as a queued `Opened` the setup pass reported through `observe_window`. The second delivery is redundant. It is not the gap replay the boot ordering exists for — nothing changed at that moment.

The gap replay is a different, smaller thing and it stays. The window observers are installed before the snapshot is read, so a window that opens in the interval fires `kAXWindowCreatedNotification` and is reported as `Opened`; it may also land in the table before the snapshot reads it. That double-delivery is genuine and is handled idempotently by the model exactly as `refactors/past/seed-at-construction.md` describes (`set_front_app` and window records assign rather than accumulate). That case is rare and correct. The per-open-window replay is neither, and it is the only thing this change removes.

This is the window analogue of the front-app fix in `seed-at-construction.md`: a value known at boot belongs in construction, and an event means something changed. `freddie_app_nav::watch` already reports only changes, never the currently-frontmost app, so the app watcher needs no boot event. `freddie_windows` reports every existing window because `observe_window` announces what it discovers; splitting discovery from announce makes the window watcher behave like the app watcher at boot.

## Change 1: split discovery from announce in `observe_window`

`observe_window` keeps its two call sites but stops announcing. A new `report_open` does the announce, and only the created-window notification calls it. The setup pass calls `observe_window` alone.

`crates/freddie_windows/src/lib.rs`.

`observe_window` loses the final `report`:

```rust
// before
/// Watch one window: record its element, subscribe to what it does, and report it open.
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
    let Some(frame) = window_frame(element) else {
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
    state.report(WindowChange::Opened(WindowFrame { window, frame }));
}
```

```rust
// after
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
```

`on_notification`'s created-window arm records the window, then announces it:

```rust
// before
    if name == kAXWindowCreatedNotification {
        observe_window(&state, registration.observer, refcon, element);
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
```

```rust
// after
    if name == kAXWindowCreatedNotification {
        observe_window(&state, registration.observer, refcon, element);
        report_open(&state, element);
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
```

The setup-pass call site in `observe_app` is unchanged; it already calls `observe_window` and nothing else:

```rust
    for window in app_windows(app_element) {
        observe_window(state, observer, refcon, window.raw());
    }
```

After this, the only `Opened` the model can see at boot is a window that opened between the observers being installed and the snapshot being read. Every window present when the observers went in is in the snapshot and reported nowhere.

## Test

`freddie_windows` has no test that asserts the boot `Opened` replay, so nothing there needs updating; the behavior removed was never pinned.

A run confirms it end to end. With mercury started fresh, the daemon log after `initial state` holds the constructed window set in that one record and no per-window `dispatch event=Window(.. Opened ..)` lines following it. Before this change there is one such line per open window; after it there are none, unless a window genuinely opened during boot.
