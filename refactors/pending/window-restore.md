# Window restore

`r` in the resize layer puts the focused window back where it was before mercury placed it.

Mercury keeps, per window, the frame the window had the last time it was somewhere mercury did not put it. Maximizing a window and pressing `r` gives back the size it had before; maximizing, then left-halving, then `r` gives back that same pre-maximize frame, because neither placement was the user's own.

The store lives in `freddie_windows`, beside `MONITORS`, keyed by `CGWindowID`. Mercury's model stays a pure function of state and event: it asks for `MercuryEffect::RestoreWindow` and knows nothing about frames or ids.

## Identity

A window's identity is its `CGWindowID`. An `AXUIElementRef` is not it: `place` creates a fresh element for the focused window on every call, and two elements for the same window are different pointers.

`_AXUIElementGetWindow` is the only way across. It is private, exported by HIServices, and has been there since 10.x. A window whose id cannot be read is placed as it is today and gets no store entry, so a future removal of the symbol costs restore and nothing else.

## Which frame is the restore frame

Two frames are kept per window:

- `restore`, where the window goes back to.
- `placed`, where mercury's last placement actually left it.

Before placing, `place` reads the window's current frame. If it matches `placed`, mercury put the window there and `restore` stays as it is. Otherwise the window is somewhere mercury did not put it, and that frame becomes the new `restore`.

`placed` is read back after the set rather than taken from the target: apps clamp what they are given (a terminal snaps to whole character cells), so the frame a window settles at is near the target, not equal to it. The comparison is per-edge within two points for the same reason.

`restore` removes the entry. The window is back where it started, so there is nothing left to put it back to.

## Dead windows

`CGWindowID`s are reused. Every write to the store first drops entries for ids that are no longer in `CGWindowListCreate`'s list, so an id handed to a new window cannot restore it to a closed window's frame. That is one Core Graphics call on a path that already costs tens of milliseconds and already runs on its own thread.

---

# Change 1: read the whole frame, not just the origin

`place` reads only the position today, to pick a monitor. Restore needs the size too, and both come from the same shaped call. No behavior change.

Before:

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
/// Read one `AXValue` attribute of `window` into `out`, which names the type to unwrap:
/// a `CGPoint` for `kAXValueTypeCGPoint`, a `CGSize` for `kAXValueTypeCGSize`.
fn ax_value<T: Copy>(
    window: AXUIElementRef,
    attribute: &str,
    kind: AXValueType,
    mut out: T,
) -> Option<T> {
    let attribute = CFString::new(attribute);
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

/// The focused window's frame, in Accessibility coordinates, or `None` if either half
/// of it cannot be read.
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

`monitor_for` takes the frame the caller already read rather than reading one itself, so `place` reads it once.

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

`place` threads the frame through:

```rust
pub fn place(placement: Placement) -> Result<(), WindowError> {
    let window = focused_window().ok_or(WindowError::NoFocusedWindow)?;

    let monitor = monitor_for(window_frame(window)).ok_or(WindowError::NotInitialized)?;
    let target = placement.within(monitor.visible);
    set_frame(window, target);

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    tracing::debug!(?placement, ?target, "placed the focused window");
    Ok(())
}
```

Imports gained: `kAXValueTypeCGSize` and `AXValueType` from `accessibility_sys`. `CGSize` is already imported.

Add to the test module:

```rust
#[test]
fn a_frame_is_read_as_its_origin_and_size() {
    let f = Frame {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 400.0,
    };
    assert!(f.contains(10.0, 20.0));
    assert!(f.contains(309.0, 419.0));
    assert!(!f.contains(310.0, 20.0));
}
```

# Change 2: a window's id

No behavior change: `place` logs the id it read.

Add to `crates/freddie_windows/src/lib.rs`:

```rust
use core_graphics::window::{CGWindowID, kCGNullWindowID};

/// A window's `CGWindowID`: the identity that outlives any one `AXUIElement` naming it.
/// [`place`] creates a fresh element for the focused window on every call, so the element
/// itself cannot be the key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct WindowId(CGWindowID);

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
/// any other and simply never enters the restore store.
fn window_id(window: AXUIElementRef) -> Option<WindowId> {
    let mut id: CGWindowID = kCGNullWindowID;
    // SAFETY: `window` is a live element; the call writes at most one `CGWindowID` into
    // `id` and takes no ownership of either.
    #[expect(unsafe_code)]
    let status = unsafe { _AXUIElementGetWindow(window, &raw mut id) };
    (status == 0 && id != kCGNullWindowID).then_some(WindowId(id))
}
```

In `place`, after the placement:

```rust
    tracing::debug!(?placement, ?target, id = ?window_id(window), "placed the focused window");
```

placed above the `CFRelease`, since it reads the element.

# Change 3: the restore store

Add to `crates/freddie_windows/src/lib.rs`:

```rust
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use core_graphics::window::{create_window_list, kCGWindowListOptionAll};

impl Frame {
    /// How far apart two frames may be and still count as the same placement, in points.
    /// Apps clamp what they are given (a terminal snaps to whole character cells), so a
    /// frame read back after a set is near what was asked for rather than equal to it.
    const TOLERANCE: f64 = 2.0;

    /// Whether this frame is `other` to within [`TOLERANCE`](Self::TOLERANCE) on every edge.
    fn approx_eq(self, other: Self) -> bool {
        (self.x - other.x).abs() <= Self::TOLERANCE
            && (self.y - other.y).abs() <= Self::TOLERANCE
            && (self.width - other.width).abs() <= Self::TOLERANCE
            && (self.height - other.height).abs() <= Self::TOLERANCE
    }
}

/// A window [`place`] has moved.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Tracked {
    /// Where [`restore`] puts the window back: the frame it had the last time [`place`]
    /// found it somewhere no placement had put it.
    restore: Frame,
    /// Where the last placement left the window, read back rather than taken from the
    /// target. Compared against the frame the next placement finds, which is how "the
    /// user moved this" is told from "mercury put it here".
    placed: Frame,
}

/// Every window [`place`] has moved and not yet put back. Written by [`place`], drained
/// by [`restore`], and pruned of dead windows on both.
static TRACKED: LazyLock<RwLock<HashMap<WindowId, Tracked>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Drop entries for windows that no longer exist. `CGWindowID`s are reused, so without
/// this an id handed to a new window would restore it to a closed window's frame.
fn prune(tracked: &mut HashMap<WindowId, Tracked>) {
    let Some(live) = create_window_list(kCGWindowListOptionAll, kCGNullWindowID) else {
        return;
    };
    let live: HashSet<WindowId> = live.iter().map(|id| WindowId(*id)).collect();
    tracked.retain(|id, _| live.contains(id));
}

/// Record where `id` goes back to, and where this placement left it.
///
/// The restore frame only moves when the window was somewhere no placement had put it, so
/// maximizing and then left-halving still restores to the frame the window had before the
/// maximize.
fn track(id: WindowId, before: Frame, landed: Frame) {
    let Ok(mut tracked) = TRACKED.write() else {
        return;
    };
    prune(&mut tracked);
    let restore = match tracked.get(&id) {
        Some(entry) if entry.placed.approx_eq(before) => entry.restore,
        _ => before,
    };
    tracked.insert(
        id,
        Tracked {
            restore,
            placed: landed,
        },
    );
}

/// Take `id`'s entry, leaving nothing behind: a restored window is where it started, so
/// there is nothing left to put it back to.
fn take(id: WindowId) -> Option<Tracked> {
    let mut tracked = TRACKED.write().ok()?;
    prune(&mut tracked);
    tracked.remove(&id)
}
```

`place` records what it did:

```rust
pub fn place(placement: Placement) -> Result<(), WindowError> {
    let window = focused_window().ok_or(WindowError::NoFocusedWindow)?;

    let before = window_frame(window);
    let monitor = monitor_for(before).ok_or(WindowError::NotInitialized)?;
    let target = placement.within(monitor.visible);
    set_frame(window, target);

    let id = window_id(window);
    if let (Some(id), Some(before), Some(landed)) = (id, before, window_frame(window)) {
        track(id, before, landed);
    }

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    tracing::debug!(?placement, ?target, ?id, ?before, "placed the focused window");
    Ok(())
}
```

`restore` is the new public half:

```rust
/// Put the focused window back where it was before it was placed.
///
/// The frame it goes to is the one it had the last time [`place`] found it somewhere no
/// placement had put it, so a run of placements all restore to the same frame. Restoring
/// forgets the window, so a second restore in a row is
/// [`WindowError::NothingToRestore`].
///
/// Immediate, with no animation, and as costly as [`place`].
///
/// # Errors
///
/// [`WindowError::NoFocusedWindow`] if nothing is frontmost or the frontmost app has no
/// focused window, and [`WindowError::NothingToRestore`] if that window has no remembered
/// frame.
pub fn restore() -> Result<(), WindowError> {
    let window = focused_window().ok_or(WindowError::NoFocusedWindow)?;

    let entry = window_id(window).and_then(take);
    if let Some(entry) = entry {
        set_frame(window, entry.restore);
    }

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }

    match entry {
        Some(entry) => {
            tracing::debug!(frame = ?entry.restore, "restored the focused window");
            Ok(())
        }
        None => Err(WindowError::NothingToRestore),
    }
}
```

The new error variant:

```rust
pub enum WindowError {
    /// [`init`] was not called, or it failed.
    NotInitialized,
    /// [`init`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// There is no screen to place a window on.
    NoScreen,
    /// Nothing is frontmost, or the frontmost app has no focused window.
    NoFocusedWindow,
    /// The focused window has no remembered frame: nothing placed it, or it has already
    /// been put back.
    NothingToRestore,
}
```

```rust
            Self::NoFocusedWindow => "no focused window",
            Self::NothingToRestore => "nothing to restore",
```

Tests, in the existing module:

```rust
#[test]
fn approx_eq_absorbs_an_app_clamping_what_it_was_given() {
    let asked = Frame {
        x: 0.0,
        y: 25.0,
        width: 800.0,
        height: 900.0,
    };
    let landed = Frame {
        width: 799.0,
        ..asked
    };
    assert!(asked.approx_eq(landed), "a point of clamping is the same frame");
    assert!(
        !asked.approx_eq(Frame {
            width: 400.0,
            ..asked
        }),
        "a half-width window is not the same frame"
    );
}

/// The restore frame survives a run of placements and only moves when the window turns
/// up somewhere no placement put it. This is `track`'s rule, spelled without the store.
#[test]
fn the_restore_frame_follows_the_user_and_not_the_placements() {
    let original = Frame {
        x: 100.0,
        y: 100.0,
        width: 400.0,
        height: 300.0,
    };
    let maximized = Frame {
        x: 0.0,
        y: 25.0,
        width: 1600.0,
        height: 900.0,
    };
    let restore_after = |entry: Option<Tracked>, before: Frame| match entry {
        Some(e) if e.placed.approx_eq(before) => e.restore,
        _ => before,
    };

    let first = restore_after(None, original);
    assert_eq!(first, original);

    let entry = Tracked {
        restore: first,
        placed: maximized,
    };
    assert_eq!(
        restore_after(Some(entry), maximized),
        original,
        "placing a placed window keeps the frame the user last had"
    );

    let dragged = Frame {
        x: 700.0,
        ..original
    };
    assert_eq!(
        restore_after(Some(entry), dragged),
        dragged,
        "a window the user moved restores to where they left it"
    );
}
```

# Change 4: `r` restores, in the resize layer

`crates/mercury/src/effect.rs`:

```rust
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
    /// Put the focused window back where it was before it was placed. A no-op if nothing
    /// placed it.
    RestoreWindow,
```

`crates/mercury/src/handlers/resize.rs`:

```rust
pub(crate) fn restore_window<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(node.parent, MercuryEffect::RestoreWindow)
}
```

Restoring is one choice, not something to repeat, so it leaves for home like the arrows.

`crates/mercury/src/state/resize.rs`:

```rust
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
    Key::KeyR.down() => restore_window,
```

`crates/mercury/src/state/overlays/resize.txt`:

```
  RESIZE
  ────────────────────
  ↑    maximize
  ←    left half
  →    right half
  r    restore
  t    typing
  esc  home
```

`crates/mercury/src/daemon.rs`, beside `MercuryEffect::Place`:

```rust
        MercuryEffect::Place(placement) => place_window(placement),
        MercuryEffect::RestoreWindow => restore_window(),
```

```rust
/// Put the focused window back, fire-and-forget on its own thread, for the same reason
/// [`place_window`] uses one.
fn restore_window() {
    std::thread::spawn(|| match freddie_windows::restore() {
        Ok(()) => debug!("restored the window"),
        Err(e) => debug!(error = %e, "nothing to restore"),
    });
}
```

`debug!` rather than `warn!`: pressing `r` on a window mercury never placed is an ordinary miss, not a failure the user has to see.

`crates/mercury/tests/transitions.rs`:

```rust
// `r` in resize is the fourth placement choice: it puts the window back and, like the
// arrows, returns home.
#[test]
fn resize_r_restores_the_window_and_returns_home() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(m.layer(), Layer::Resize(_)));

    assert_eq!(
        m.handle(&key(Key::KeyR)),
        Some(leaves(vec![MercuryEffect::RestoreWindow]))
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// `r r` is enter-resize then restore, so a second `r` does not re-enter resize.
#[test]
fn r_in_resize_does_not_re_enter_resize() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::KeyR));
    assert!(matches!(m.layer(), Layer::Home(_)));
}
```

Extend the resize keymap table with every key the layer answers, `r` among them:

```rust
#[test]
fn resize_answers_exactly_its_keymap() {
    for (k, effects) in [
        (Key::UpArrow, vec![MercuryEffect::Place(Placement::Maximize)]),
        (Key::LeftArrow, vec![MercuryEffect::Place(Placement::LeftHalf)]),
        (
            Key::RightArrow,
            vec![MercuryEffect::Place(Placement::RightHalf)],
        ),
        (Key::KeyR, vec![MercuryEffect::RestoreWindow]),
    ] {
        let mut m = home();
        let _ = m.handle(&key(Key::KeyR));
        assert_eq!(m.handle(&key(k)), Some(leaves(effects)), "{k:?}");
        assert!(matches!(m.layer(), Layer::Home(_)), "{k:?} stayed in resize");
    }
}
```
