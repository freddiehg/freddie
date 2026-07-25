# a placement shrinks, moves, then grows

A placement writes position and size twice, unconditionally:

```rust
fn set_frame(window: AXUIElementRef, frame: Frame) {
    let origin = CGPoint::new(frame.x, frame.y);
    let size = CGSize::new(frame.width, frame.height);

    for _ in 0..2 {
        set_attribute::<Position>(window, origin);
        set_attribute::<Size>(window, size);
    }
}
```

Four `AXUIElementSetAttributeValue` calls, each an IPC round trip into the app that owns the window, which is where the tens of milliseconds a placement costs comes from. The second pass exists because a single pass can clamp: position and size are separate writes, an app validates each against the value the other currently holds, and the intermediate state between them can cover more than either the start or the target. A window 600 wide at x=1000 asked for 1600 wide at x=1000 is clamped if the size lands first, because 1000 plus 1600 leaves the screen.

There is an order that never produces such an intermediate, and it does not depend on the screen, the number of monitors, or which screen anybody is measuring against:

1. Shrink each axis that needs to shrink.
2. Move.
3. Grow each axis that needs to grow.

Every intermediate is then contained in either the start rectangle or the target rectangle. Shrinking at the old origin covers no more than the start, which the window already occupies. Moving at the shrunk size covers no more than the target on each axis, because each axis is already at or below its target extent. Growing happens at the target origin, so the last step is the target itself. Both the start and the target fit, since the start is where the window is and the target was computed from a visible frame, so no step asks for a rectangle that does not.

It costs two writes when a placement only shrinks or only grows, and three when one axis shrinks while the other grows. Never four.

Doing it needs the size the window currently has. The crate computes exactly that on every report, in `report_open` and in the moved and resized branch, and throws it away after handing it to the consumer.

## Shape after

The table keeps the last frame it reported beside the element, and `set_frame` takes the frame the window is at as well as the one it is going to.

```rust
/// A window being watched: the element to address it through, and where it was last reported to
/// be.
///
/// The frame is kept because a placement needs the size the window currently has to order its
/// writes, and this is already computed for every report. It is the same mirror of external
/// truth the rest of the table is: seeded from the snapshot, then replaced by whatever the moved
/// and resized notifications say.
struct Watched {
    element: Element,
    frame: Frame,
}

/// Every window that can be addressed, the element to address it through, and where it is.
#[derive(Default)]
struct Elements(Mutex<HashMap<WindowId, Arc<Watched>>>);
```

```rust
/// Move and resize one window, in an order that cannot be clamped.
///
/// Shrink, move, grow. Position and size are separate writes and an app validates each against
/// the value the other one holds, so the intermediate state between two writes has to fit as
/// well as the endpoints do. Shrinking first keeps the intermediate inside `from`, which the
/// window already occupies; moving at the shrunk size keeps it inside `to` on both axes; growing
/// happens once the origin is already right. Nothing here consults a screen, because containment
/// in `from` or `to` is what makes each step safe and both of those fit by construction.
///
/// Two writes for a pure shrink or a pure grow, three when one axis goes each way.
fn set_frame(window: AXUIElementRef, from: Frame, to: Frame) {
    let origin = CGPoint::new(to.x, to.y);
    let shrunk = CGSize::new(from.width.min(to.width), from.height.min(to.height));
    let target = CGSize::new(to.width, to.height);

    if shrunk.width < from.width || shrunk.height < from.height {
        set_attribute::<Size>(window, shrunk);
    }
    set_attribute::<Position>(window, origin);
    if target.width > shrunk.width || target.height > shrunk.height {
        set_attribute::<Size>(window, target);
    }
}
```

A stale `from` cannot break it. If the frame the table holds is smaller than the window really is, the shrink asks for less than it needed to and every later step is still bounded by `to`. If it is larger, the first write is a grow at the old origin which an app may clamp, and a clamp there only leaves the window smaller than asked, which the move tolerates and the final write corrects at the target origin. The two steps that must not be clamped, the move and the final size, are bounded by `to` either way.

## Change 0: a failed attribute write says so

File: `crates/freddie_windows/src/lib.rs`. Independent of the rest.

`set_attribute` discards the `AXError`, so a clamped or refused write is indistinguishable from one that landed. That is what makes the current behaviour unverifiable, and the whole point of the ordering is to avoid a clamp, so a clamp needs to be visible.

### Before

```rust
/// Set one `AXValue` attribute of `element`.
fn set_attribute<A: AxAttribute>(element: AXUIElementRef, value: A::Value) {
    // SAFETY: `AXValueCreate` copies out of the pointer it is given, which lives for the
    // call, and returns a +1 reference `Owned` takes responsibility for.
    #[expect(unsafe_code)]
    let Some(boxed) =
        (unsafe { Owned::new(AXValueCreate(A::KIND, (&raw const value).cast()).cast()) })
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

### After

```rust
/// Set one `AXValue` attribute of `element`.
///
/// A failure is logged and skipped rather than returned: a placement is three of these and there
/// is nothing useful for a caller to do with a partial one. The log is what says whether a write
/// landed, which is how the ordering in [`set_frame`] is checked.
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
        tracing::debug!(attribute = A::NAME, status, "an attribute write was refused");
    }
}
```

`debug` rather than `warn` for the refusal, because an app declining a frame is the app's decision and mercury keeps running; the file records it either way.

## Change 1: the table keeps the frame it last reported

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 0 only for reading the log during verification.

### `Element` gains a wrapper

`Element` and its `raw` are unchanged. `Watched` is new, as written in "Shape after", and the table holds `Arc<Watched>`.

### `observe_window` records the frame it reads

Before:

```rust
    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(window, Arc::new(Element(owned)));
    }
```

After:

```rust
    // Read here rather than carried from `report_open`, which reads it again for the event: the
    // two are one call apart and the element is live for both. A frame that cannot be read has no
    // default worth inventing, since a placement would then order its writes from a lie, so the
    // window is not recorded at all.
    let Some(frame) = window_frame(element) else {
        return;
    };
    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(
            window,
            Arc::new(Watched {
                element: Element(owned),
                frame,
            }),
        );
    }
```

This moves one behaviour: a window whose position or size cannot be read is no longer added to the table, so it cannot be placed. `report_open` already declines to announce such a window, so nothing downstream ever knew it existed.

### The moved and resized branch replaces the frame

Before:

```rust
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
        if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
            let moved = WindowFrame { window, frame };
            state.report(if name == kAXWindowMovedNotification {
                WindowChange::Moved(moved)
            } else {
                WindowChange::Resized(moved)
            });
        }
```

After:

```rust
    } else if name == kAXWindowMovedNotification || name == kAXWindowResizedNotification {
        if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
            state.record(window, frame);
            let moved = WindowFrame { window, frame };
            state.report(if name == kAXWindowMovedNotification {
                WindowChange::Moved(moved)
            } else {
                WindowChange::Resized(moved)
            });
        }
```

with the recorder on `WatcherState`:

```rust
    /// Replace where `window` is understood to be. Idempotent, like every report of external
    /// truth: it assigns and never accumulates.
    ///
    /// A window not in the table is not added, because a frame without an element cannot be
    /// placed through.
    fn record(&self, window: WindowId, frame: Frame) {
        if let Ok(mut table) = self.elements.0.lock()
            && let Some(watched) = table.get_mut(&window)
        {
            *watched = Arc::new(Watched {
                element: watched.element.retained(),
                frame,
            });
        }
    }
```

`Element::retained` is needed because the `Arc` is replaced rather than mutated, and `Owned` is deliberately not `Clone`:

```rust
impl Element {
    /// A second owned reference to the same element.
    ///
    /// `CFRetain` rather than deriving `Clone` on [`Owned`], which two references naming one
    /// element would release twice.
    fn retained(&self) -> Self {
        // SAFETY: `self` holds a live +1 reference, so retaining it yields a second one, which
        // the returned `Element` releases on drop.
        #[expect(unsafe_code)]
        let raw = unsafe { CFRetain(self.raw().cast()) };
        Self(Owned(raw))
    }
}
```

### Every other table access

`forget` and `forget_app` are unchanged apart from the value type. `forget_element` reaches the element through the wrapper:

```rust
            .find(|(_, held)| unsafe { CFEqual(held.element.raw().cast(), element.cast()) != 0 })
```

The snapshot in `watch` reads the stored frame instead of asking the OS for it again, which removes two IPC round trips per window at startup:

```rust
    let windows: Vec<WindowFrame> = state.elements.0.lock().map_or_else(
        |_| Vec::new(),
        |table| {
            table
                .iter()
                .map(|(window, watched)| WindowFrame {
                    window: *window,
                    frame: watched.frame,
                })
                .collect()
        },
    );
```

## Change 2: order the writes

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 1 for the stored frame.

`set_frame` takes `from` as written in "Shape after", and `WindowSink::set_frame` reads it out of the entry it already looked up:

```rust
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        let elements = self.elements.upgrade().ok_or(WindowError::NotWatching)?;
        // Cloned out so the lock is released before the writes: those take tens of
        // milliseconds, and the main thread takes this lock every time a window opens or
        // closes.
        let watched = {
            let table = elements.0.lock().map_err(|_| WindowError::UnknownWindow)?;
            Arc::clone(
                table
                    .get(&target.window)
                    .ok_or(WindowError::UnknownWindow)?,
            )
        };
        set_frame(watched.element.raw(), watched.frame, target.frame);
        tracing::debug!(?target, from = ?watched.frame, "set a window's frame");
        Ok(())
    }
```

## Tests

`set_frame` is FFI, but the ordering is arithmetic and belongs in a table. Extract it so the decision is testable without a window server:

```rust
/// The writes a placement performs, in order. `None` for a size means that write is skipped.
#[derive(PartialEq, Debug)]
struct Writes {
    shrink: Option<CGSize>,
    origin: CGPoint,
    grow: Option<CGSize>,
}

fn writes_for(from: Frame, to: Frame) -> Writes {
    let shrunk = CGSize::new(from.width.min(to.width), from.height.min(to.height));
    let target = CGSize::new(to.width, to.height);
    Writes {
        shrink: (shrunk.width < from.width || shrunk.height < from.height).then_some(shrunk),
        origin: CGPoint::new(to.x, to.y),
        grow: (target.width > shrunk.width || target.height > shrunk.height).then_some(target),
    }
}
```

`set_frame` becomes a loop over what `writes_for` returned, and the tests are the table:

```rust
    const FROM: Frame = Frame { x: 1000.0, y: 100.0, width: 600.0, height: 400.0 };

    // Growing while moving left: nothing to shrink, so the move goes first at the old size and
    // the grow lands at the target origin.
    #[test]
    fn a_pure_grow_moves_before_it_grows() {
        let to = Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, None);
        assert_eq!(w.origin, CGPoint::new(0.0, 0.0));
        assert_eq!(w.grow, Some(CGSize::new(1600.0, 900.0)));
    }

    // Shrinking while moving right: the shrink goes first, so the intermediate never reaches
    // past the target's right edge.
    #[test]
    fn a_pure_shrink_shrinks_before_it_moves() {
        let to = Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, Some(CGSize::new(400.0, 300.0)));
        assert_eq!(w.grow, None);
    }

    // One axis each way: both writes happen, the shrink covering only the axis that shrinks.
    #[test]
    fn a_mixed_change_shrinks_then_grows() {
        let to = Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, Some(CGSize::new(400.0, 400.0)));
        assert_eq!(w.grow, Some(CGSize::new(400.0, 900.0)));
    }

    // A frame that is already right is one write, and it is the move.
    #[test]
    fn an_unchanged_size_is_only_a_move() {
        let to = Frame { x: 0.0, y: 0.0, ..FROM };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, None);
        assert_eq!(w.grow, None);
    }

    // Every intermediate is inside `from` or inside `to` on both axes, which is what makes the
    // order safe without consulting a screen.
    #[test]
    fn no_intermediate_exceeds_both_endpoints() {
        for to in [
            Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 },
            Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 },
            Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 },
        ] {
            let w = writes_for(FROM, to);
            if let Some(shrink) = w.shrink {
                assert!(FROM.x + shrink.width <= FROM.x + FROM.width);
                assert!(FROM.y + shrink.height <= FROM.y + FROM.height);
            }
            let moved = w.shrink.unwrap_or(CGSize::new(FROM.width, FROM.height));
            assert!(to.x + moved.width <= to.x + to.width);
            assert!(to.y + moved.height <= to.y + to.height);
        }
    }
```

## Call sites

None outside `crates/freddie_windows/src/lib.rs`. `WindowSink::set_frame` keeps its signature, `WindowChange` and `Snapshot` are unchanged, and mercury needs no edit.

`refactors/pending/placements-go-through-a-channel.md` touches the same table. If that lands first, `Elements` is a `RefCell<HashMap<WindowId, Watched>>` with no `Arc`, `Watcher::pump` does the lookup and passes `watched.frame` as `from`, and `Element::retained` is already defined there.

## Verification

```
cargo test -p freddie_windows
```

The `writes_for` table above covers the ordering. The writes themselves need a window server:

1. Place a window that grows and moves left, then one that shrinks and moves right, from the resize layer. Both land exactly on the target rather than short of it.
2. The log shows two or three `an attribute write was refused` lines fewer than before, which is to say none, on placements that used to need the second pass to converge.
3. A placement is measurably shorter. Time from the `SetFrame` effect record to the `set a window's frame` record drops by roughly a quarter to a half, being one or two IPC round trips out of four.

## Ordered commits

1. Change 0: `set_attribute` logs a refused write.
2. Change 1: `Watched` holds the element and its last reported frame; `record` replaces it on move and resize; the snapshot reads it rather than the OS.
3. Change 2: `writes_for` decides the order, `set_frame` performs shrink, move, grow, and the tests table it.
