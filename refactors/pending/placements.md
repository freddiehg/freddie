# a placement is a message, and it shrinks before it moves

Two problems in one path, and each one's fix is what the other one needs.

## The table is shared across threads

`freddie_windows` is the only crate in the family that shares mutable state. The element table is an `Arc<Mutex<HashMap<..>>>`, a `WindowSink` holds a `Weak` to it, and the values are `Arc<Element>` so one can be cloned out from under the lock:

```rust
struct Elements(Mutex<HashMap<WindowId, Arc<Element>>>);

pub struct WindowSink {
    elements: Weak<Elements>,
}
```

All three exist because the table is reachable from two threads. The AX callbacks write it on the main thread; `WindowSink::set_frame` reads it from whatever thread the caller is on. Nothing else in the crate needs them: `WatcherState::apps` is a bare `RefCell` because it is main-thread only, and the table is the odd one out.

It is shared rather than owned because neither end may block. The table belongs to the main thread because that is where the observers deliver. The AX write must not run there, because main is a serialized doorman and a write costing tens of milliseconds would stall every other source.

## A placement writes twice, blindly

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

Four `AXUIElementSetAttributeValue` calls, each an IPC round trip into the app that owns the window, which is where the tens of milliseconds comes from. The second pass exists because a single pass can clamp: position and size are separate writes, an app validates each against the value the other currently holds, and the intermediate between them can cover more than either endpoint. A window 600 wide at x=1000 asked for 1600 wide at the same origin is clamped if the size lands first, because 1000 plus 1600 leaves the screen.

There is an order that never produces such an intermediate:

1. Shrink each axis that needs to shrink.
2. Move.
3. Grow each axis that needs to grow.

Every intermediate is then contained in either the start rectangle or the target rectangle. Shrinking at the old origin covers no more than the start, which the window already occupies. Moving at the shrunk size covers no more than the target, because each axis is already at or below its target extent. Growing happens at the target origin, so the last step is the target itself. Both endpoints fit by construction, so no step asks for a rectangle that does not, and no screen geometry enters the argument.

Two writes for a pure shrink or a pure grow, three when one axis goes each way. Never four.

## Why they are one change

Ordering the writes needs the size the window currently has. Moving the lookup to the thread that owns the table is what puts that size within reach: the entry is right there, and the crate already computes a frame for every report and throws it away. So the channel gives the ordering its `from`, and the ordering gives the channel's `pump` something better to do than what `set_frame` did before.

Separately, `set_attribute` discards the `AXError`, so a clamped write is indistinguishable from one that landed. That is what makes the current behaviour unverifiable, and avoiding a clamp is the point, so a clamp has to be visible.

## Shape after

The table is a plain main-thread map of window to element and last known frame. The sink is a sender. The main thread drains placements on each wake, as `freddie_overlay` already does with `Overlay::pump`, and hands each write to a thread of its own.

```rust
/// A window being watched: the element to address it through, and where it was last reported to
/// be.
///
/// The frame is kept because a placement needs the size the window currently has in order to
/// order its writes, and it is already computed for every report. It is the same mirror of
/// external truth as the rest of the table: seeded at construction, then replaced by whatever the
/// moved and resized notifications say.
struct Watched {
    element: Element,
    frame: Frame,
}

/// Every window that can be addressed, the element to address it through, and where it is.
///
/// Main-thread only, like `apps`: the AX callbacks that write it and the `pump` that reads it both
/// run there, so there is nothing to lock.
type Elements = HashMap<WindowId, Watched>;

struct WatcherState {
    /// Every window being watched. A `RefCell` and not a `Mutex`: nothing off the main thread
    /// reaches it.
    elements: RefCell<Elements>,
    /// One entry per observed app. Held here rather than on the [`Watcher`] because the launch
    /// and terminate callbacks are `'static` closures that cannot borrow it.
    apps: RefCell<HashMap<Pid, AppObserver>>,
    on_change: Box<dyn Fn(WindowChange)>,
}

/// The handle a placement is performed through.
///
/// Cheap to clone and unattached to the thread that made it: it is a sender, and the placement is
/// looked up and performed by the thread that owns the table.
///
/// Dropping the [`Watcher`] drops the receiver, so a send from a sink that has outlived its
/// watcher is how [`WindowError::NotWatching`] is answered.
#[derive(Clone)]
pub struct WindowSink {
    placements: WakingSender<WindowFrame>,
}

/// Holds every registration that makes windows report, and the placement queue.
pub struct Watcher {
    /// The workspace and screen observations. Held for their `Drop`, and declared first so they
    /// stop before the state they write into is torn down: fields drop in declaration order.
    _notifications: Vec<Observation>,
    /// Handed to every [`WindowSink`].
    placements_sender: WakingSender<WindowFrame>,
    /// Placements waiting to be performed. Drained by [`Watcher::pump`] on the main thread.
    placements: Receiver<WindowFrame>,
    state: Rc<WatcherState>,
}
```

`Element` is stored by value rather than behind an `Arc`. `unsafe impl Send for Owned` stays, because the element still crosses to the thread doing the write, but as an explicit move rather than as access to a shared table; `unsafe impl Sync for Owned` goes, because nothing shares one.

## Change 0: a failed attribute write says so

File: `crates/freddie_windows/src/lib.rs`. Independent of everything below.

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
/// A refusal is logged and skipped rather than returned: a placement is two or three of these and
/// there is nothing useful for a caller to do with a partial one. The log is what says whether a
/// write landed, which is how the ordering in [`set_frame`] is checked.
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

`debug` rather than `warn` for a refusal: an app declining a frame is the app's decision and mercury keeps running. The file records it either way.

## Change 1: placements go through a channel

Files: `crates/freddie_windows/src/lib.rs` and `crates/mercury/src/daemon.rs`. Both in one commit, because `watch` gains a parameter.

### `Elements` and `WindowSink::set_frame`

Before:

```rust
#[derive(Default)]
struct Elements(Mutex<HashMap<WindowId, Arc<Element>>>);

#[derive(Clone)]
pub struct WindowSink {
    elements: Weak<Elements>,
}

impl WindowSink {
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        let elements = self.elements.upgrade().ok_or(WindowError::NotWatching)?;
        let element = {
            let table = elements.0.lock().map_err(|_| WindowError::UnknownWindow)?;
            Arc::clone(
                table
                    .get(&target.window)
                    .ok_or(WindowError::UnknownWindow)?,
            )
        };
        set_frame(element.raw(), target.frame);
        tracing::debug!(?target, "set a window's frame");
        Ok(())
    }
}
```

After:

```rust
type Elements = HashMap<WindowId, Element>;

#[derive(Clone)]
pub struct WindowSink {
    placements: WakingSender<WindowFrame>,
}

impl WindowSink {
    /// Move and resize one window: `target` names which, and the rectangle it goes to.
    ///
    /// Queues the placement and wakes the main thread, which owns the element table. The write
    /// costs tens of milliseconds and runs on a thread of its own, so this returns immediately and
    /// a caller on a latency-sensitive loop needs no thread of its own.
    ///
    /// The frame is the caller's, already worked out. This does not consult the screen, the
    /// frontmost app, or anything else.
    ///
    /// # Errors
    ///
    /// [`WindowError::NotWatching`] if the watcher has been dropped. A window that is not being
    /// observed cannot be reported here, because the lookup happens after the send; [`Watcher::pump`]
    /// logs it at `debug` instead.
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        self.placements
            .send(target)
            .map_err(|_| WindowError::NotWatching)
    }
}
```

### `Watcher::pump`

New, and the counterpart to `Overlay::pump`.

```rust
impl Watcher {
    /// A handle to perform placements through. Cheap to clone, `Send`, and safe to keep past the
    /// watcher, which it answers [`WindowError::NotWatching`] from.
    #[must_use]
    pub fn sink(&self) -> WindowSink {
        WindowSink {
            placements: self.placements_sender.clone(),
        }
    }

    /// Perform every placement queued since the last wake.
    ///
    /// On the main thread, because that is where the element table lives. The lookup is a hashmap
    /// hit; the write is handed to a thread of its own, because it costs tens of milliseconds and
    /// this thread is what every other source is waiting on.
    pub fn pump(&self) {
        for target in self.placements.try_iter() {
            let Some(element) = self
                .state
                .elements
                .borrow()
                .get(&target.window)
                .map(Element::retained)
            else {
                tracing::debug!(?target, "no such window to place");
                continue;
            };
            std::thread::spawn(move || {
                set_frame(element.raw(), target.frame);
                tracing::debug!(?target, "set a window's frame");
            });
        }
    }
}
```

`Element::retained` is what lets the spawned thread own its element rather than borrow the table:

```rust
impl Element {
    /// A second owned reference to the same element, for handing to another thread.
    ///
    /// `CFRetain` rather than deriving `Clone` on [`Owned`], which two values naming one element
    /// would release twice.
    fn retained(&self) -> Self {
        // SAFETY: `self` holds a live +1 reference, so retaining it yields a second one, which the
        // returned `Element` releases on drop.
        #[expect(unsafe_code)]
        let raw = unsafe { CFRetain(self.raw().cast()) };
        Self(Owned(raw))
    }
}
```

### `watch`

Before:

```rust
pub fn watch(
    on_change: impl Fn(WindowChange) + 'static,
) -> Result<(Watcher, Snapshot), WindowError> {
    // ...
    let state = Rc::new(WatcherState {
        elements: Arc::new(Elements::default()),
        apps: RefCell::new(HashMap::new()),
        on_change: Box::new(on_change),
    });
```

After:

```rust
pub fn watch(
    waker: &MainWaker,
    on_change: impl Fn(WindowChange) + 'static,
) -> Result<(Watcher, Snapshot), WindowError> {
    // ...
    let state = Rc::new(WatcherState {
        elements: RefCell::new(HashMap::new()),
        apps: RefCell::new(HashMap::new()),
        on_change: Box::new(on_change),
    });
    let (placements_sender, placements) = waker.channel::<WindowFrame>();
```

and it returns

```rust
    Ok((
        Watcher {
            _notifications: notifications,
            placements_sender,
            placements,
            state,
        },
        snapshot,
    ))
```

### Every other table access

Each loses its lock. `forget`:

```rust
    fn forget(&self, window: WindowId) -> bool {
        self.elements.borrow_mut().remove(&window).is_some()
    }
```

`forget_element`:

```rust
    fn forget_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        let mut table = self.elements.borrow_mut();
        // SAFETY: both are live `AXUIElement`s as far as CoreFoundation is concerned. A destroyed
        // element is still a valid CF object; it is the Accessibility calls on it that fail.
        #[expect(unsafe_code)]
        let found = table
            .iter()
            .find(|(_, held)| unsafe { CFEqual(held.raw().cast(), element.cast()) != 0 })
            .map(|(id, _)| *id)?;
        table.remove(&found);
        Some(found)
    }
```

`observe_window`'s insert:

```rust
    state.elements.borrow_mut().insert(window, Element(owned));
```

`forget_app`'s scan, where the `borrow` must end before the loop that calls `forget` and `report`, or it is a `BorrowMutError`:

```rust
    let gone: Vec<WindowId> = state
        .elements
        .borrow()
        .iter()
        .filter(|(_, element)| window_id(element.raw()).is_none())
        .map(|(id, _)| *id)
        .collect();
    for window in gone {
        if state.forget(window) {
            state.report(WindowChange::Closed(window));
        }
    }
```

That strictness is in the direction that fails loudly on the first run rather than quietly in production, which the `Mutex` did not offer.

The snapshot in `watch`:

```rust
    let windows: Vec<WindowFrame> = state
        .elements
        .borrow()
        .iter()
        .filter_map(|(window, element)| {
            window_frame(element.raw()).map(|frame| WindowFrame {
                window: *window,
                frame,
            })
        })
        .collect();
```

### Imports

`use std::sync::{Arc, Mutex, Weak};` goes. `freddie_main_loop::{MainWaker, WakingSender}` and `std::sync::mpsc::Receiver` come in. `std::cell::RefCell` and `core_foundation::base::CFRetain` are already imported. `unsafe impl Sync for Owned` is deleted, and the comment on `unsafe impl Send for Owned` changes to say the element is moved to the placement thread rather than shared through a lock.

### mercury

`watch` takes the waker, which `run` already holds:

```rust
    let windows = freddie_windows::watch(&waker, {
        let event_tx = event_tx.clone();
        move |change| {
            let _ = event_tx.send(MercuryEvent::Window(WindowEvent { change }));
        }
    });
```

`_window_watcher` is read now rather than only held, so it is renamed `window_watcher`, and the main loop drains placements beside the overlay:

```rust
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
        overlay.pump();
        if let Some(watcher) = window_watcher.as_ref() {
            watcher.pump();
        }
    });
```

The effect loses its helper, because `set_frame` no longer blocks. Before:

```rust
        MercuryEffect::SetFrame(target) => set_frame(windows, target),
```

```rust
/// Set one window's frame, fire-and-forget on its own thread. It takes tens of
/// milliseconds, which is long enough to delay a key the effect loop is about to emit. A
/// detached thread cannot hold up the exit path the way `spawn_blocking` would, which is
/// the same reason `foreground_app` uses one.
fn set_frame(windows: Option<&WindowSink>, target: WindowFrame) {
    let Some(windows) = windows.cloned() else {
        debug!(?target, "no window sink: nothing to place through");
        return;
    };
    std::thread::spawn(move || match windows.set_frame(target) {
        Ok(()) => debug!(?target, "set the window's frame"),
        Err(e) => warn!(?target, error = %e, "set frame failed"),
    });
}
```

After, the helper is deleted and the arm queues directly:

```rust
        MercuryEffect::SetFrame(target) => match windows {
            Some(windows) => {
                if let Err(e) = windows.set_frame(target) {
                    warn!(?target, error = %e, "set frame failed");
                }
            }
            None => debug!(?target, "no window sink: nothing to place through"),
        },
```

## Change 2: the table keeps the frame it last reported

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 1.

`Watched` is new, as written in "Shape after", and the map's value becomes one. `observe_window` records the frame it reads:

```rust
    // Read here rather than carried from `report_open`, which reads it again for the event: the
    // two are one call apart and the element is live for both. A frame that cannot be read has no
    // default worth inventing, since a placement would then order its writes from a lie, so the
    // window is not recorded at all.
    let Some(frame) = window_frame(element) else {
        return;
    };
    state.elements.borrow_mut().insert(
        window,
        Watched {
            element: Element(owned),
            frame,
        },
    );
```

That moves one behaviour: a window whose position or size cannot be read is no longer in the table, so it cannot be placed. `report_open` already declines to announce such a window, so nothing downstream ever knew it existed.

The moved and resized branch replaces the stored frame. Before:

```rust
        if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
            let moved = WindowFrame { window, frame };
```

After:

```rust
        if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
            state.record(window, frame);
            let moved = WindowFrame { window, frame };
```

with

```rust
    /// Replace where `window` is understood to be. Idempotent, like every report of external
    /// truth: it assigns and never accumulates.
    ///
    /// A window not in the table is not added, because a frame without an element cannot be placed
    /// through.
    fn record(&self, window: WindowId, frame: Frame) {
        if let Some(watched) = self.elements.borrow_mut().get_mut(&window) {
            watched.frame = frame;
        }
    }
```

`forget_element` reaches through the wrapper, and `forget_app`'s filter with it:

```rust
            .find(|(_, held)| unsafe { CFEqual(held.element.raw().cast(), element.cast()) != 0 })
```

```rust
        .filter(|(_, watched)| window_id(watched.element.raw()).is_none())
```

`pump` takes the element out of it:

```rust
                .map(|watched| watched.element.retained())
```

And the snapshot reads the stored frame rather than asking the OS again, which removes two IPC round trips per window at startup:

```rust
    let windows: Vec<WindowFrame> = state
        .elements
        .borrow()
        .iter()
        .map(|(window, watched)| WindowFrame {
            window: *window,
            frame: watched.frame,
        })
        .collect();
```

## Change 3: order the writes

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 2 for the stored frame.

The decision is arithmetic, so it comes out of the FFI and into something testable:

```rust
/// The writes one placement performs, in order. A `None` size is a write that is skipped.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Writes {
    shrink: Option<CGSize>,
    origin: CGPoint,
    grow: Option<CGSize>,
}

/// Shrink, move, grow.
///
/// Position and size are separate writes and an app validates each against the value the other one
/// holds, so the intermediate between two writes has to fit as well as the endpoints do. Shrinking
/// first keeps the intermediate inside `from`, which the window already occupies. Moving at the
/// shrunk size keeps it inside `to` on both axes. Growing happens once the origin is already right,
/// so the last write is `to` itself. Nothing here consults a screen, because containment in `from`
/// or `to` is what makes each step safe and both of those fit by construction.
fn writes_for(from: Frame, to: Frame) -> Writes {
    let shrunk = CGSize::new(from.width.min(to.width), from.height.min(to.height));
    let target = CGSize::new(to.width, to.height);
    Writes {
        shrink: (shrunk.width < from.width || shrunk.height < from.height).then_some(shrunk),
        origin: CGPoint::new(to.x, to.y),
        grow: (target.width > shrunk.width || target.height > shrunk.height).then_some(target),
    }
}

/// Move and resize one window, in an order that cannot be clamped. See [`writes_for`].
///
/// Two writes for a pure shrink or a pure grow, three when one axis goes each way. A stale `from`
/// cannot break it: too small under-shrinks and every later step is still bounded by `to`, and too
/// large makes the first write a grow that an app may clamp, which only leaves the window smaller
/// than asked. The two writes that must not be clamped, the move and the final size, are bounded
/// by `to` either way.
fn set_frame(window: AXUIElementRef, from: Frame, to: Frame) {
    let Writes {
        shrink,
        origin,
        grow,
    } = writes_for(from, to);
    if let Some(size) = shrink {
        set_attribute::<Size>(window, size);
    }
    set_attribute::<Position>(window, origin);
    if let Some(size) = grow {
        set_attribute::<Size>(window, size);
    }
}
```

`pump` passes both frames:

```rust
            let Some((element, from)) = self
                .state
                .elements
                .borrow()
                .get(&target.window)
                .map(|watched| (watched.element.retained(), watched.frame))
            else {
                tracing::debug!(?target, "no such window to place");
                continue;
            };
            std::thread::spawn(move || {
                set_frame(element.raw(), from, target.frame);
                tracing::debug!(?target, ?from, "set a window's frame");
            });
```

### Tests

```rust
    const FROM: Frame = Frame { x: 1000.0, y: 100.0, width: 600.0, height: 400.0 };

    // Growing while moving left: nothing to shrink, so the move goes first at the old size and the
    // grow lands at the target origin.
    #[test]
    fn a_pure_grow_moves_before_it_grows() {
        let to = Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, None);
        assert_eq!(w.origin, CGPoint::new(0.0, 0.0));
        assert_eq!(w.grow, Some(CGSize::new(1600.0, 900.0)));
    }

    // Shrinking while moving right: the shrink goes first, so the intermediate never reaches past
    // the target's right edge.
    #[test]
    fn a_pure_shrink_shrinks_before_it_moves() {
        let to = Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, Some(CGSize::new(400.0, 300.0)));
        assert_eq!(w.grow, None);
    }

    // One axis each way: both size writes happen, and the first shrinks only the axis that shrinks.
    #[test]
    fn a_mixed_change_shrinks_then_grows() {
        let to = Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, Some(CGSize::new(400.0, 400.0)));
        assert_eq!(w.grow, Some(CGSize::new(400.0, 900.0)));
    }

    // A frame that is already the right size is one write, and it is the move.
    #[test]
    fn an_unchanged_size_is_only_a_move() {
        let to = Frame { x: 0.0, y: 0.0, ..FROM };
        let w = writes_for(FROM, to);
        assert_eq!(w.shrink, None);
        assert_eq!(w.grow, None);
    }

    // The invariant the order rests on: every intermediate sits inside `from` or inside `to` on
    // both axes, which is why no screen is consulted.
    #[test]
    fn no_intermediate_exceeds_its_endpoint() {
        for to in [
            Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 },
            Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 },
            Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 },
            Frame { x: 0.0, y: 0.0, ..FROM },
        ] {
            let w = writes_for(FROM, to);
            if let Some(shrink) = w.shrink {
                assert!(shrink.width <= FROM.width && shrink.height <= FROM.height);
            }
            let moved = w.shrink.unwrap_or(CGSize::new(FROM.width, FROM.height));
            assert!(moved.width <= to.width && moved.height <= to.height);
        }
    }
```

## Call sites

`freddie_windows`'s public surface changes in two places: `watch` takes a `&MainWaker` first, and `Watcher` gains `pump`. `WindowSink::set_frame`, `WindowChange`, `Snapshot`, `WindowFrame` and `WindowId` keep their shapes; `set_frame` changes only when it returns.

Mercury is the only consumer here and its edits are in Change 1. figaro takes `freddie_windows` by path and needs the same two.

`WindowError::UnknownWindow` keeps its variant and loses this producer; the seed path still uses the rest of the enum.

## Verification

```
cargo test -p freddie_windows -p mercury
cargo clippy --all-targets --all-features
```

`writes_for` is covered by the table above. The rest needs a window server and the Accessibility grant, so by hand after `mercury restart`:

1. Place a window that grows and moves left, then one that shrinks and moves right, from the resize layer. Both land exactly on the target rather than short of it.
2. `mercury logs --level debug` shows no `an attribute write was refused` lines on placements that used to need the second pass to converge.
3. Placement does not stall the keyboard. Hold a key while placing: keys keep emitting at the usual rate, because the write is still off the main thread and off the effect loop.
4. Place a window that has just closed, which used to answer `UnknownWindow`: the log shows `no such window to place` and nothing else happens.
5. Window changes still arrive at all, which is what confirms the table is written correctly through its new `RefCell`: open and close a window and see `Opened` and `Closed`.
6. A placement is measurably shorter. Time from the `SetFrame` dispatch record to `set a window's frame` drops by roughly a quarter to a half, being one or two IPC round trips saved out of four.

## Ordered commits

1. Change 0: `set_attribute` logs a refused write.
2. Change 1: `WindowSink` becomes a sender, the table becomes a main-thread `RefCell`, `Watcher::pump` performs placements, `Arc`, `Mutex` and `Weak` leave the crate, and mercury pumps and drops its helper.
3. Change 2: `Watched` holds the element and its last reported frame, `record` replaces it on move and resize, and the snapshot reads it rather than the OS.
4. Change 3: `writes_for` decides the order, `set_frame` performs shrink, move, grow, and the tests table it.
