# Window restore

`r` in the resize layer puts the focused window back where it was before mercury placed it.

Mercury keeps, per window, the frame it had when the first placement moved it. Maximizing and pressing `r` gives back the size it had before. Maximizing, then left-halving, then `r` gives back that same pre-maximize frame, because the second placement did not move the window away from anywhere the user chose. Moving the window by hand forgets the remembered frame: it is where the user wants it, so there is nothing to put it back to.

All of it is `Mercury.windows`. The handler reads state, the effect is the `SetFrame` that already exists, and the whole rule is checkable in `transitions.rs`.

Depends on `refactors/past/window-observation.md` and `refactors/pending/placement-in-the-model.md`.

## Telling mercury's move from the user's

`kAXWindowMovedNotification` and `kAXWindowResizedNotification` fire the same way whether a drag moved the window or mercury did, so the model has to know which of its own placements is in flight.

`Windows::pending` is that: the window a `SetFrame` was just asked for, the frame it asked for, and a timer guard. While it is set, every move reported for that window is mercury's own and the remembered frame is left alone. It clears when a reported frame matches the one asked for, and otherwise when the timer fires.

The timer is what makes it safe rather than a guess. `set_frame` writes the position and the size, twice, so one placement produces up to four reports, and the intermediate ones are of frames nobody asked for. Waiting for the matching frame alone would strand `pending` forever on an app that clamps beyond tolerance (a terminal snapping to whole character cells), and every later drag of that window would be read as mercury's. The timer bounds it: after `PLACEMENT_SETTLE` the window is the user's again whatever arrived.

The match is per-edge within two points, for the same clamping reason.

---

# Change 1: the remembered frame

`crates/mercury/src/state/` gains it, on the `Windows` that `refactors/past/window-observation.md` introduced.

Before:

```rust
#[derive(Debug, Default)]
pub struct Windows {
    /// Every open window and where it is.
    frames: HashMap<WindowId, Frame>,
    /// The focused window, `None` when nothing is focused or its id is unreadable.
    focused: Option<WindowId>,
    /// The monitors, in the order the source reported them.
    screens: Vec<Monitor>,
}
```

After:

```rust
#[derive(Debug, Default)]
pub struct Windows {
    /// Every open window: where it is, and where it goes back to.
    windows: HashMap<WindowId, WindowState>,
    /// The focused window, `None` when nothing is focused or its id is unreadable.
    focused: Option<WindowId>,
    /// The monitors, in the order the source reported them.
    screens: Vec<Monitor>,
    /// The placement mercury has asked for and not yet seen land. See
    /// [`PendingPlacement`].
    pending: Option<PendingPlacement>,
}

/// One window: where it is now, and where a restore would put it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct WindowState {
    /// Where the window is, as the source last reported it.
    frame: Frame,
    /// Where the window was before mercury first moved it. `None` once it is back there,
    /// or once the user has moved it since.
    restore: Option<Frame>,
}

/// A [`MercuryEffect::SetFrame`] that has been asked for and not yet landed.
///
/// While one is outstanding, every move reported for its window is mercury's own doing,
/// so the remembered frame survives it. One placement produces several reports, and only
/// the last is the frame that was asked for.
#[derive(Debug)]
struct PendingPlacement {
    window: WindowId,
    /// The frame the effect asked for. A report matching it ends the wait.
    frame: Frame,
    /// Held for its `Drop` and for the trigger that matches its firing: the wait ends
    /// when this fires, whatever has been reported.
    timer: TimerGuard,
}

/// How long a placement has to land before the window is the user's again.
///
/// It bounds how long a drag can be mistaken for mercury's own placement, so shorter is
/// better, but it has to cover two position-and-size writes and the reports they produce.
pub const PLACEMENT_SETTLE: Duration = Duration::from_millis(250);
```

The comparison lives in mercury, beside the rule it serves. `freddie_windows::Frame` stays four numbers: how close two frames have to be before a placement counts as landed is this consumer's policy, and a different one would pick a different number or want exactness.

```rust
/// How far apart two frames may be and still be the same placement, in points.
///
/// Apps clamp what they are given (a terminal snaps to whole character cells), so a frame
/// reported after a set is near what was asked for rather than equal to it.
const TOLERANCE: f64 = 2.0;

/// Whether `a` is `b` to within [`TOLERANCE`] on every edge.
fn same_placement(a: Frame, b: Frame) -> bool {
    (a.x - b.x).abs() <= TOLERANCE
        && (a.y - b.y).abs() <= TOLERANCE
        && (a.width - b.width).abs() <= TOLERANCE
        && (a.height - b.height).abs() <= TOLERANCE
}
```

# Change 2: recording a move decides whether to forget

`Windows::record`'s frame-change arms, before:

```rust
            WindowChange::Moved(moved) | WindowChange::Resized(moved) => {
                self.frames.insert(moved.window, moved.frame);
            }
```

After:

```rust
            WindowChange::Moved(moved) | WindowChange::Resized(moved) => {
                let ours = self.pending_covers(moved);
                if let Some(state) = self.windows.get_mut(&moved.window) {
                    state.frame = moved.frame;
                    if !ours {
                        state.restore = None;
                    }
                }
            }
```

```rust
impl Windows {
    /// Whether `moved` is a report of mercury's own outstanding placement, ending the
    /// wait if it is the frame that was asked for.
    ///
    /// Every report for the pending window counts, not only the matching one: one
    /// placement writes the position and the size, twice, so the frames in between are
    /// ones nobody asked for and a drag they were mistaken for would be forgotten.
    fn pending_covers(&mut self, moved: WindowFrame) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        if pending.window != moved.window {
            return false;
        }
        if same_placement(pending.frame, moved.frame) {
            self.pending = None;
        }
        true
    }
}
```

The timer's firing clears it, bound at the root beside the other guard-matched triggers:

```rust
    // Only the placement still outstanding: a firing from one already landed matches nothing.
    |mercury_path| mercury_path.windows.pending_timer().map(TimerGuard::trigger) => placement_settled,
```

```rust
/// The placement mercury asked for has had its time: whatever the window has done since,
/// what it does next is the user's.
pub(crate) fn placement_settled(
    _ev: &TimerFired,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.windows.forget_pending();
    Vec::new()
}
```

`Closed` drops the whole entry, so a `CGWindowID` handed to a new window arrives with no remembered frame:

```rust
            WindowChange::Closed(window) => {
                self.windows.remove(window);
                if self.focused == Some(*window) {
                    self.focused = None;
                }
            }
```

# Change 3: a placement remembers, and arms the wait

`crates/mercury/src/handlers/resize.rs`, before:

```rust
fn place<'a, P: Ascend<MercuryPath<'a>>>(path: P, placement: Placement) -> Vec<MercuryEffect> {
    let root = path.ascend();
    let effects = match target(&root.windows, placement) {
        Some(target) => vec![MercuryEffect::SetFrame(target)],
        None => Vec::new(),
    };
    and_go_home(root, effects)
}
```

After:

```rust
fn place<'a, P: Ascend<MercuryPath<'a>>>(path: P, placement: Placement) -> Vec<MercuryEffect> {
    let root = path.ascend();
    let effects = match target(&root.windows, placement) {
        Some(target) => root.windows.placing(target),
        None => Vec::new(),
    };
    and_go_home_from(root, effects)
}
```

```rust
impl Windows {
    /// Record that `target` is about to be asked for, and return the effects that ask:
    /// the placement itself and the timer that bounds the wait for it.
    ///
    /// The frame the window has now becomes the one a restore goes back to, unless one is
    /// already remembered: a run of placements all restore to where the window was before
    /// the first of them.
    pub(crate) fn placing(&mut self, target: WindowFrame) -> Vec<MercuryEffect> {
        let Some(state) = self.windows.get_mut(&target.window) else {
            return Vec::new();
        };
        state.restore = state.restore.or(Some(state.frame));

        let (timer, effect) = timer_effect_and_guard(PLACEMENT_SETTLE, |id| {
            MercuryEvent::Timer(TimerFired(id))
        });
        self.pending = Some(PendingPlacement {
            window: target.window,
            frame: target.frame,
            timer,
        });
        vec![MercuryEffect::SetFrame(target), MercuryEffect::Timer(effect)]
    }
}
```

# Change 4: `r` restores

```rust
impl Windows {
    /// Take the focused window's remembered frame, and return the effects that put it
    /// back. Empty when nothing is focused or the window has no remembered frame: nothing
    /// placed it, or it is already back.
    ///
    /// Taking, not reading: a restored window is where it started, so there is nothing
    /// left to put it back to.
    pub(crate) fn restoring(&mut self) -> Vec<MercuryEffect> {
        let Some(window) = self.focused else {
            return Vec::new();
        };
        let Some(frame) = self.windows.get_mut(&window).and_then(|s| s.restore.take()) else {
            return Vec::new();
        };
        self.placing_without_remembering(WindowFrame { window, frame })
    }
}
```

`placing_without_remembering` is `placing` minus the `state.restore` line: a restore still arms the wait, so the moves it causes are not read as the user's, but it has nothing to remember.

The handler, beside the three arrows:

```rust
/// Put the focused window back where it was, and return home.
///
/// Restoring is one choice, not something to repeat, so it leaves the layer the way the
/// arrows do.
pub(crate) fn restore_window<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent.ascend();
    let effects = root.windows.restoring();
    and_go_home_from(root, effects)
}
```

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

# Change 5: tests

In `crates/mercury/tests/transitions.rs`, on top of `home_with_a_window` from `refactors/pending/placement-in-the-model.md`. A placement now produces two effects, the `SetFrame` and the settle timer, so the existing resize assertions gain the timer.

```rust
// `r` in resize puts the window back where it was before the placement, and returns home
// like the arrows.
#[test]
fn resize_r_restores_the_frame_from_before_the_placement() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));
    let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
        window: WINDOW,
        frame: SCREEN.visible,
    })));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        Some(leaves(vec![
            MercuryEffect::SetFrame(WindowFrame {
                window: WINDOW,
                frame: WINDOW_FRAME,
            }),
            settle_timer(),
        ]))
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

// A run of placements restores to where the window was before the first of them, not to
// the frame the previous placement left.
#[test]
fn a_second_placement_does_not_move_the_remembered_frame() { ... }

// A move mercury did not ask for forgets the remembered frame, so `r` afterwards does
// nothing rather than dragging the window off where the user just put it.
#[test]
fn a_move_by_hand_forgets_the_remembered_frame() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));
    let effects = ...;
    let _ = m.handle(&fired(timer_id(&effects)));

    let dragged = Frame { x: 700.0, ..WINDOW_FRAME };
    let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
        window: WINDOW,
        frame: dragged,
    })));

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(m.handle(&key(Key::KeyR)), Some(leaves(vec![])));
}

// The reports a single placement produces are the position and the size, each written
// twice, so the frames in between are ones nobody asked for. None of them counts as a
// move by hand.
#[test]
fn the_intermediate_frames_of_a_placement_are_not_a_move_by_hand() {
    let mut m = home_with_a_window();
    let _ = m.handle(&key(Key::KeyR));
    let _ = m.handle(&key(Key::UpArrow));

    for frame in [
        // The position landed, the size has not.
        Frame { x: 0.0, y: 25.0, ..WINDOW_FRAME },
        SCREEN.visible,
    ] {
        let _ = m.handle(&windows(WindowChange::Moved(WindowFrame {
            window: WINDOW,
            frame,
        })));
    }

    let _ = m.handle(&key(Key::KeyR));
    assert_eq!(
        m.handle(&key(Key::KeyR)),
        Some(leaves(vec![
            MercuryEffect::SetFrame(WindowFrame {
                window: WINDOW,
                frame: WINDOW_FRAME,
            }),
            settle_timer(),
        ]))
    );
}

// An app that clamps past the tolerance never reports the frame that was asked for, so
// the wait ends on the timer instead and the next drag is the user's again.
#[test]
fn a_placement_that_never_lands_settles_on_the_timer() { ... }

// Restoring takes the frame, so a second `r` has nothing to put back.
#[test]
fn restoring_twice_asks_for_nothing_the_second_time() { ... }

// `r r` is enter-resize then restore, so a second `r` does not re-enter resize.
#[test]
fn r_in_resize_does_not_re_enter_resize() { ... }

// A closed window takes its remembered frame with it, so a reused `CGWindowID` cannot
// restore a new window to a closed one's frame.
#[test]
fn a_closed_window_is_forgotten() { ... }
```
