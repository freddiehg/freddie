# Placement in the model

The resize handlers compute the frame a window is going to, and the effect carries it. `MercuryEffect::Place(Placement)` becomes `MercuryEffect::SetFrame(WindowFrame)`: a window id and a rectangle, with nothing left to work out.

Today the effect names an intent and the effect handler works out the rest, reading the frontmost app, the focused window, and the monitor list at effect time. After this, dispatch reads `Mercury.windows`, which `refactors/pending/window-observation.md` fills, and the effect handler does one thing: it sets that frame on that window.

Behavior does not change. `r` then an arrow places the focused window exactly where it does now, and the resize layer's keymap is untouched.

The visible gain is in the tests: `the_arrows_place_the_window_and_return_home` currently asserts an intent, so nothing checks that maximize means the visible frame or that the halves abut. With the windows and screens in the model, the transition tests assert the rectangle.

Depends on `refactors/pending/window-observation.md`. Without it `Mercury.windows` is always empty and every placement is a no-op.

---

# Change 1: the placement arithmetic moves to mercury

`Placement` stops being an effect payload and becomes what the three resize handlers share, so it moves out of `crates/mercury/src/effect.rs` into `crates/mercury/src/handlers/resize.rs` and gains the arithmetic `freddie_windows` is doing today.

Removed from `crates/mercury/src/effect.rs`:

```rust
/// Where a window should go. Mercury's own, mirroring `freddie_windows::Placement` so the
/// model stays free of the OS crates, the way `App` is free of bundle ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Maximize,
    LeftHalf,
    RightHalf,
}
```

Added to `crates/mercury/src/handlers/resize.rs`:

```rust
/// Where a window goes, as a share of the screen it is on. One variant per resize key.
///
/// Not an effect payload: a handler turns it into the rectangle the effect carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Placement {
    /// The whole visible frame.
    Maximize,
    LeftHalf,
    RightHalf,
}

impl Placement {
    /// The frame this placement occupies within `visible`.
    const fn within(self, visible: Frame) -> Frame {
        let half = visible.width / 2.0;
        match self {
            Self::Maximize => visible,
            Self::LeftHalf => Frame {
                width: half,
                ..visible
            },
            Self::RightHalf => Frame {
                x: visible.x + half,
                width: half,
                ..visible
            },
        }
    }
}
```

The `within` tests move with it, out of `freddie_windows`'s test module and into `handlers/resize.rs`: `maximize_is_the_whole_visible_frame`, `the_halves_split_the_width_and_keep_the_height`, `the_halves_abut`, and `placements_are_relative_to_the_visible_frame`.

# Change 2: the effect carries the rectangle

`crates/mercury/src/effect.rs`:

Before:

```rust
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
```

After:

```rust
    /// Move and resize one window, named by id, to a rectangle already worked out.
    ///
    /// The sink does not ask what is frontmost, what is focused, or what the screen looks
    /// like. The handler that produced this read all of it out of the model.
    SetFrame(WindowFrame),
```

The three handlers in `crates/mercury/src/handlers/resize.rs`:

Before:

```rust
pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    and_go_home(node.parent, MercuryEffect::Place(Placement::Maximize))
}
```

After:

```rust
pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    place(node.parent, Placement::Maximize)
}
```

with `left_half` and `right_half` the same shape, and the one function they share:

```rust
/// Put the focused window where `placement` says, and return home.
///
/// The effects are empty when there is no focused window or no screen has been reported.
/// The layer returns home either way.
fn place<'a, P: Ascend<MercuryPath<'a>>>(path: P, placement: Placement) -> Vec<MercuryEffect> {
    let root = path.ascend();
    let effects = match target(&root.windows, placement) {
        Some(target) => vec![MercuryEffect::SetFrame(target)],
        None => Vec::new(),
    };
    and_go_home(root, effects)
}

/// The window a placement moves and the frame it moves it to.
fn target(windows: &Windows, placement: Placement) -> Option<WindowFrame> {
    let focused = windows.focused()?;
    let monitor = windows.monitor_for(focused.frame)?;
    Some(WindowFrame {
        window: focused.window,
        frame: placement.within(monitor.visible),
    })
}
```

`and_go_home` takes `impl Into<Vec<MercuryEffect>>` already, so an empty vector needs nothing new. It takes a path today and a `&mut Mercury` here; the signature widens to take the root directly, since `place` has already ascended:

Before:

```rust
pub(crate) fn and_go_home<'a, P: Ascend<MercuryPath<'a>>>(
    path: P,
    effects: impl Into<Vec<MercuryEffect>>,
) -> Vec<MercuryEffect> {
    let mut effects = effects.into();
    effects.extend(go_home(path.ascend()));
    effects
}
```

After:

```rust
pub(crate) fn and_go_home<'a, P: Ascend<MercuryPath<'a>>>(
    path: P,
    effects: impl Into<Vec<MercuryEffect>>,
) -> Vec<MercuryEffect> {
    and_go_home_from(path.ascend(), effects)
}

/// [`and_go_home`] for a caller that has already ascended.
pub(crate) fn and_go_home_from(
    root: &mut Mercury,
    effects: impl Into<Vec<MercuryEffect>>,
) -> Vec<MercuryEffect> {
    let mut effects = effects.into();
    effects.extend(go_home(root));
    effects
}
```

# Change 3: the sink stops deciding

`crates/freddie_windows/src/lib.rs` loses `place`, `Placement`, `monitor_for`, and `focused_window`, all of which existed to work out what the model now works out. `WindowSink::set_frame` from `refactors/pending/window-observation.md` is the whole sink.

`MONITORS` goes with them. It is a cache of main-thread-only `NSScreen` data that exists because `place` runs off the main thread, and `place` is its only reader; the model holds `screens` now, so the model is the cache. `read_monitors` stays as a plain function, called by the screen-change observer to build a `Screens` report and by `Watcher::snapshot` for the seed.

`init` is left holding the Accessibility check and the no-screen check, small enough to fold into `watch`. `freddie_windows` ends up with no statics.

`crates/mercury/src/daemon.rs`:

Before:

```rust
        MercuryEffect::Place(placement) => place_window(placement),
```

```rust
fn place_window(placement: Placement) {
    let placement = match placement {
        Placement::Maximize => freddie_windows::Placement::Maximize,
        Placement::LeftHalf => freddie_windows::Placement::LeftHalf,
        Placement::RightHalf => freddie_windows::Placement::RightHalf,
    };
    std::thread::spawn(move || match freddie_windows::place(placement) {
        Ok(()) => debug!(?placement, "placed the window"),
        Err(e) => warn!(?placement, error = %e, "place failed"),
    });
}
```

After:

```rust
        MercuryEffect::SetFrame(target) => set_frame(windows, target),
```

```rust
/// Set one window's frame, fire-and-forget on its own thread. It takes tens of
/// milliseconds, which is long enough to delay a key the effect loop is about to emit. A
/// detached thread cannot hold up the exit path the way `spawn_blocking` would, which is
/// the same reason `foreground_app` uses one.
///
/// The sink is a clone of the one the `Watcher` handed out, which is what the thread
/// carries instead of reaching for a global.
fn set_frame(windows: &WindowSink, target: WindowFrame) {
    let windows = windows.clone();
    std::thread::spawn(move || match windows.set_frame(target.window, target.frame) {
        Ok(()) => debug!(?target, "set the window's frame"),
        Err(e) => warn!(?target, error = %e, "set frame failed"),
    });
}
```

The effect loop gains the `WindowSink` alongside the `Emitter` it already carries, taken from the `Watcher` the daemon holds for the life of the process.

The `Placement` translation goes away with it: the mirrored enum existed so the model could name a placement without depending on the OS crate, and there is no placement to name any more.

# Change 4: the transition tests assert rectangles

`crates/mercury/tests/transitions.rs` gains a seeded root, since a placement now needs a focused window and a screen:

```rust
// One 1600x900 screen with a 25pt menu bar, and one small window focused on it. Enough for
// a placement to have somewhere to go.
const SCREEN: Monitor = Monitor {
    full: Frame { x: 0.0, y: 0.0, width: 1600.0, height: 925.0 },
    visible: Frame { x: 0.0, y: 25.0, width: 1600.0, height: 900.0 },
};
const WINDOW: WindowId = WindowId(7);
const WINDOW_FRAME: Frame = Frame { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };

// A mercury in Home that has been told about one screen and one focused window.
fn home_with_a_window() -> Mercury {
    let mut m = home();
    let _ = m.handle(&windows(WindowChange::Screens(vec![SCREEN])));
    let _ = m.handle(&windows(WindowChange::Opened(WindowFrame {
        window: WINDOW,
        frame: WINDOW_FRAME,
    })));
    let _ = m.handle(&windows(WindowChange::Focused(Some(WINDOW))));
    m
}
```

Before:

```rust
        assert_eq!(
            m.handle(&key(k)),
            Some(leaves(vec![MercuryEffect::Place(placement)])),
            "{k:?}"
        );
```

After:

```rust
    for (k, frame) in [
        (Key::UpArrow, SCREEN.visible),
        (Key::LeftArrow, Frame { width: 800.0, ..SCREEN.visible }),
        (Key::RightArrow, Frame { x: 800.0, width: 800.0, ..SCREEN.visible }),
    ] {
        let mut m = home_with_a_window();
        let _ = m.handle(&key(Key::KeyR));

        assert_eq!(
            m.handle(&key(k)),
            Some(leaves(vec![MercuryEffect::SetFrame(WindowFrame {
                window: WINDOW,
                frame,
            })])),
            "{k:?}"
        );
    }
```

And the cases that used to be unreachable from a test:

```rust
// With nothing focused there is nothing to place, so the key is spent and the layer is
// left, but no window moves.
#[test]
fn a_placement_with_no_focused_window_asks_for_nothing() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::UpArrow)), Some(leaves(vec![])));
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// A window on the second display fills that display, not the one mercury started on.
#[test]
fn a_placement_uses_the_screen_the_window_is_on() { ... }
```
