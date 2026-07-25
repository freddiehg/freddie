# a placement shrinks before it moves, and is a message

Two things about placing a window, ordered so the smaller and more valuable one lands first.

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

Four `AXUIElementSetAttributeValue` calls, each an IPC round trip into the app that owns the window, which is where the tens of milliseconds a placement costs comes from. Two or three of those four do the work.

The second pass is there for a clamp that has not been reproduced. Position and size are separate writes, so the theory is that an app validates each against the value the other currently holds and the intermediate between them can cover more than either endpoint. Measured against Finder on a 3840-wide screen, that does not happen: a single pass landed exactly on target in both orders when growing and moving left, and Finder accepted 800 wide at x=3400 and x=3400 while 1600 wide, both of which put the window well off screen. The only clamp observable there is a minimum width, 483 in place of every 400 asked for, which is order-independent and which no number of passes fixes.

So the reason to change this is the round trips, and the order below is what makes dropping to one pass safe if an app that clamps this way does exist. It costs nothing to be in the right order.

The order that never produces such an intermediate:

1. Shrink each axis that needs to shrink.
2. Move.
3. Grow each axis that needs to grow.

Every intermediate is then contained in either the start rectangle or the target rectangle. Shrinking at the old origin covers no more than the start, which the window already occupies. Moving at the shrunk size covers no more than the target, because each axis is already at or below its target extent. Growing happens at the target origin, so the last write is the target itself. Both endpoints fit by construction, so no step asks for a rectangle that does not, and no screen geometry enters the argument.

Two writes for a pure shrink or a pure grow, three when one axis goes each way. Never four.

Doing it needs the size the window currently has. The crate computes exactly that on every report, in `report_open` and in the moved and resized branch, and throws it away.

Separately, `set_attribute` discards the `AXError`, so a clamped write is indistinguishable from one that landed. Avoiding a clamp is the point, so a clamp has to be visible.

## The table is shared across threads

`freddie_windows` is the only crate in the family that shares mutable state. The element table is an `Arc<Mutex<HashMap<..>>>`, a `WindowSink` holds a `Weak` to it, and the values are `Arc<Element>` so one can be cloned out from under the lock:

```rust
struct Elements(Mutex<HashMap<WindowId, Arc<Element>>>);

pub struct WindowSink {
    elements: Weak<Elements>,
}
```

All three exist because the table is reachable from two threads. The AX callbacks write it on the main thread; `WindowSink::set_frame` reads it from whatever thread the caller is on. Nothing else in the crate needs them: `WatcherState::apps` is a bare `RefCell` because it is main-thread only, and the table is the odd one out.

It is shared rather than owned because neither end may block. The table belongs to the main thread because that is where the observers deliver. The AX write must not run there, because main is a serialized doorman and a write costing tens of milliseconds would stall every other source. A channel satisfies both: main owns the table and does the lookup, which is a hashmap hit, and hands the element it found to a thread of its own for the write.

This is last rather than first because the write ordering does not need it. `WindowSink::set_frame` already does the lookup under the lock, so it already has the entry that carries the frame.

## Shape after

```rust
/// A width and a height.
///
/// Not `CGSize`, which does not implement `PartialEq`, and deliberately without CoreGraphics in it
/// so the write ordering is arithmetic a test can table.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Extent {
    width: f64,
    height: f64,
}

/// The size writes one placement performs, around the move that sits between them.
///
/// The move is unconditional and always goes to the target's origin, so it is not named here.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Writes {
    /// The size to shrink to before moving, when either axis shrinks.
    shrink: Option<Extent>,
    /// The size to grow to after moving, when either axis grows.
    grow: Option<Extent>,
}

/// A window being watched: the element to address it through, and where it was last reported to
/// be.
///
/// The frame is kept because a placement needs the size the window currently has in order to order
/// its writes, and it is already computed for every report. It is the same mirror of external truth
/// as the rest of the table: seeded at construction, then replaced by whatever the moved and resized
/// notifications say.
struct Watched {
    element: Element,
    frame: Frame,
}

/// Every window that can be addressed, the element to address it through, and where it is.
///
/// Main-thread only, like `apps`: the AX callbacks that write it and the `pump` that reads it both
/// run there, so there is nothing to lock.
type Elements = HashMap<WindowId, Watched>;

/// The handle a placement is performed through.
///
/// Cheap to clone and unattached to the thread that made it: it is a sender, and the placement is
/// looked up and performed by the thread that owns the table.
#[derive(Clone)]
pub struct WindowSink {
    placements: WakingSender<WindowFrame>,
}
```

## Change 0: the error enum stops advertising what it cannot return

File: `crates/freddie_windows/src/lib.rs`. Independent of everything else.

`WindowError::NoFocusedWindow` has no producer anywhere: not in this crate, not in mercury, not in figaro. It exists in the enum and in its `Display` arm and nothing constructs it.

### Before

```rust
pub enum WindowError {
    /// [`watch`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// Nothing is frontmost, or the frontmost app has no focused window.
    NoFocusedWindow,
    /// Nothing with that id is being observed: the window closed, or it never reported an
    /// id to begin with.
    UnknownWindow,
    /// The watcher has been dropped, so nothing is being observed at all.
    NotWatching,
}
```

### After

```rust
pub enum WindowError {
    /// [`watch`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// Nothing with that id is being observed: the window closed, or it never reported an
    /// id to begin with.
    UnknownWindow,
    /// The watcher has been dropped, so nothing is being observed at all.
    NotWatching,
}
```

and the matching `Display` arm goes:

```rust
            Self::NotMainThread => "freddie_windows::watch must run on the main thread",
            Self::NotTrusted => "Accessibility is not granted",
            Self::UnknownWindow => "no such window",
            Self::NotWatching => "not watching windows",
```

## Change 1: a refused attribute write warns

File: `crates/freddie_windows/src/lib.rs`. Independent.

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
///
/// `warn`, because a window that does not go where it was asked to go is visible to whoever asked.
/// An app refusing a frame it considers out of bounds, or below its minimum size, is the likeliest
/// reason a placement looks broken, and it should not take `--level debug` to find out.
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
        tracing::warn!(attribute = A::NAME, status, "an attribute write was refused");
    }
}
```

## Change 2: the table keeps the frame it last reported

File: `crates/freddie_windows/src/lib.rs`. Independent of Changes 0 and 1.

Against the table as it is today, so the value keeps its `Arc<Element>` and gains a frame beside it. The `Arc` is what lets the element be cloned out from under the lock, so it stays until Change 5 removes the lock.

### The table

```rust
/// A window being watched: the element to address it through, and where it was last reported to
/// be. See the note on `frame` in "Shape after".
struct Watched {
    element: Arc<Element>,
    frame: Frame,
}

/// Every window that can be addressed, the element to address it through, and where it is.
///
/// A `Mutex` and not an `RwLock`: a window opening and a key being pressed are both rare, so there
/// is nothing for concurrent readers to win. It is held for a lookup and a clone, never across an
/// `AXUIElement` call.
#[derive(Default)]
struct Elements(Mutex<HashMap<WindowId, Watched>>);
```

### `observe_window` records the frame it reads

Before:

```rust
    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(window, Arc::new(Element(owned)));
    }
```

After:

```rust
    // Read here rather than carried from `report_open`, which reads it again for the event: the two
    // are one call apart and the element is live for both. A frame that cannot be read has no
    // default worth inventing, since a placement would then order its writes from a lie, so the
    // window is not recorded at all.
    let Some(frame) = window_frame(element) else {
        return;
    };
    if let Ok(mut table) = state.elements.0.lock() {
        table.insert(
            window,
            Watched {
                element: Arc::new(Element(owned)),
                frame,
            },
        );
    }
```

That moves one behaviour: a window whose position or size cannot be read is no longer in the table, so it cannot be placed. `report_open` already declines to announce such a window, so nothing downstream ever knew it existed.

### The moved and resized branch replaces the frame

Before:

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

with the recorder on `WatcherState`:

```rust
    /// Replace where `window` is understood to be. Idempotent, like every report of external truth:
    /// it assigns and never accumulates.
    ///
    /// A window not in the table is not added, because a frame without an element cannot be placed
    /// through.
    fn record(&self, window: WindowId, frame: Frame) {
        let Ok(mut table) = self.elements.0.lock() else {
            return;
        };
        if let Some(watched) = table.get_mut(&window) {
            watched.frame = frame;
        }
    }
```

### The other table readers

`WindowSink::set_frame` takes the frame out alongside the element:

```rust
        let (element, from) = {
            let table = elements.0.lock().map_err(|_| WindowError::UnknownWindow)?;
            let watched = table.get(&target.window).ok_or(WindowError::UnknownWindow)?;
            (Arc::clone(&watched.element), watched.frame)
        };
        set_frame(element.raw(), target.frame);
        tracing::debug!(?target, ?from, "set a window's frame");
```

`forget_element` and `forget_app` reach through the wrapper:

```rust
            .find(|(_, held)| unsafe { CFEqual(held.element.raw().cast(), element.cast()) != 0 })
```

```rust
                .filter(|(_, watched)| window_id(watched.element.raw()).is_none())
```

And the snapshot in `watch` reads the stored frame rather than asking the OS again, which removes two IPC round trips per window at startup:

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

The free `set_frame` keeps its two-argument signature here, so this commit compiles on its own: the frame this change starts storing is read out and logged, and Change 4 is what gives `set_frame` a `from` to order its writes by.

## Change 3: a newly opened window's frame is read once

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 2.

`observe_window` now reads the frame, and `report_open` reads it again one call later, so a window opening costs four IPC round trips to learn a rectangle that has not changed between them. `observe_window` already has the value the announcement needs, so it hands it over.

### `report_open` before

```rust
/// Report a window as newly open. Its frame is read now, at announce time, rather than
/// carried from `observe_window`: the two are one call apart and the window is live for
/// both. A window whose frame cannot be read is not announced.
fn report_open(state: &WatcherState, element: AXUIElementRef) {
    if let (Some(window), Some(frame)) = (window_id(element), window_frame(element)) {
        state.report(WindowChange::Opened(WindowFrame { window, frame }));
    }
}
```

### `report_open` after

```rust
/// Report a window as newly open, with the frame [`observe_window`] recorded for it.
///
/// The frame is carried rather than read again: reading position and size is two IPC round trips
/// into the app that owns the window, `observe_window` has just made them, and nothing between the
/// two calls can have moved it.
///
/// A window that is not in the table is not announced, which is how a window whose frame could not
/// be read stays unreported: `observe_window` declined to record it.
fn report_open(state: &WatcherState, element: AXUIElementRef) {
    let Some(window) = window_id(element) else {
        return;
    };
    let Some(frame) = state.frame_of(window) else {
        return;
    };
    state.report(WindowChange::Opened(WindowFrame { window, frame }));
}
```

with the reader on `WatcherState`:

```rust
    /// Where `window` was last reported to be, if it is being watched.
    fn frame_of(&self, window: WindowId) -> Option<Frame> {
        self.elements
            .0
            .lock()
            .ok()?
            .get(&window)
            .map(|watched| watched.frame)
    }
```

Its call site in `on_notification` is unchanged, because `observe_window` runs immediately before it and is what put the frame there:

```rust
    if name == kAXWindowCreatedNotification {
        observe_window(&state, registration.observer, refcon, element);
        report_open(&state, element);
    }
```

After Change 5 the lock goes and `frame_of` becomes `self.elements.borrow().get(&window).map(|w| w.frame)`.

## Change 4: order the writes

File: `crates/freddie_windows/src/lib.rs`. Depends on Change 2.

The decision is arithmetic, so it comes out of the FFI and into something testable. `Extent` and `Writes` are as written in "Shape after".

```rust
/// Shrink, move, grow.
///
/// Position and size are separate writes and an app validates each against the value the other one
/// holds, so the intermediate between two writes has to fit as well as the endpoints do. Shrinking
/// first keeps the intermediate inside `from`, which the window already occupies. Moving at the
/// shrunk size keeps it inside `to` on both axes. Growing happens once the origin is already right,
/// so the last write is `to` itself. Nothing here consults a screen, because containment in `from`
/// or `to` is what makes each step safe and both of those fit by construction.
fn writes_for(from: Frame, to: Frame) -> Writes {
    let shrunk = Extent {
        width: from.width.min(to.width),
        height: from.height.min(to.height),
    };
    let target = Extent {
        width: to.width,
        height: to.height,
    };
    Writes {
        shrink: (shrunk.width < from.width || shrunk.height < from.height).then_some(shrunk),
        grow: (target.width > shrunk.width || target.height > shrunk.height).then_some(target),
    }
}

/// Move and resize one window, in an order that cannot be clamped. See [`writes_for`].
///
/// Two writes for a pure shrink or a pure grow, three when one axis goes each way. A stale `from`
/// cannot break it: too small under-shrinks and every later step is still bounded by `to`, and too
/// large makes the first write a grow that an app may clamp, which only leaves the window smaller
/// than asked. The two writes that must not be clamped, the move and the final size, are bounded by
/// `to` either way.
fn set_frame(window: AXUIElementRef, from: Frame, to: Frame) {
    let Writes { shrink, grow } = writes_for(from, to);
    if let Some(extent) = shrink {
        set_attribute::<Size>(window, CGSize::new(extent.width, extent.height));
    }
    set_attribute::<Position>(window, CGPoint::new(to.x, to.y));
    if let Some(extent) = grow {
        set_attribute::<Size>(window, CGSize::new(extent.width, extent.height));
    }
}
```

`WindowSink::set_frame` passes the frame it already read:

```rust
        set_frame(element.raw(), from, target.frame);
```

### Tests

Added to the existing `mod tests`, which imports `super::Frame` today and needs `super::{Extent, Writes, writes_for}` too.

```rust
    const FROM: Frame = Frame {
        x: 1000.0,
        y: 100.0,
        width: 600.0,
        height: 400.0,
    };

    const fn extent(width: f64, height: f64) -> Extent {
        Extent { width, height }
    }

    // Growing while moving left: nothing to shrink, so the move goes first at the old size and the
    // grow lands at the target origin.
    #[test]
    fn a_pure_grow_moves_before_it_grows() {
        let to = Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 };
        assert_eq!(
            writes_for(FROM, to),
            Writes { shrink: None, grow: Some(extent(1600.0, 900.0)) }
        );
    }

    // Shrinking while moving right: the shrink goes first, so the intermediate never reaches past
    // the target's right edge.
    #[test]
    fn a_pure_shrink_shrinks_before_it_moves() {
        let to = Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 };
        assert_eq!(
            writes_for(FROM, to),
            Writes { shrink: Some(extent(400.0, 300.0)), grow: None }
        );
    }

    // One axis each way: both size writes happen, and the first shrinks only the axis that shrinks.
    #[test]
    fn a_mixed_change_shrinks_then_grows() {
        let to = Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 };
        assert_eq!(
            writes_for(FROM, to),
            Writes {
                shrink: Some(extent(400.0, 400.0)),
                grow: Some(extent(400.0, 900.0)),
            }
        );
    }

    // A frame that is already the right size is one write, and it is the move.
    #[test]
    fn an_unchanged_size_is_only_a_move() {
        let to = Frame { x: 0.0, y: 0.0, ..FROM };
        assert_eq!(writes_for(FROM, to), Writes { shrink: None, grow: None });
    }

    // The invariant the order rests on: the shrink never exceeds `from` and the size the move
    // happens at never exceeds `to`, on both axes, which is why no screen is consulted.
    #[test]
    fn no_intermediate_exceeds_its_endpoint() {
        for to in [
            Frame { x: 0.0, y: 0.0, width: 1600.0, height: 900.0 },
            Frame { x: 1400.0, y: 100.0, width: 400.0, height: 300.0 },
            Frame { x: 500.0, y: 100.0, width: 400.0, height: 900.0 },
            Frame { x: 0.0, y: 0.0, ..FROM },
        ] {
            let writes = writes_for(FROM, to);
            if let Some(shrink) = writes.shrink {
                assert!(shrink.width <= FROM.width && shrink.height <= FROM.height);
            }
            let moved = writes.shrink.unwrap_or(extent(FROM.width, FROM.height));
            assert!(moved.width <= to.width && moved.height <= to.height);
        }
    }
```

## Change 5: placements go through a channel

Files: `crates/freddie_windows/src/lib.rs` and `crates/mercury/src/daemon.rs`, in one commit because `watch` gains a parameter. Depends on Changes 2 and 3.

### `Elements`, `Watched` and `WindowSink`

The lock goes, so `Arc<Element>` is no longer needed to clone out from under it, and `Watched` holds an `Element` by value. `WatcherState::elements` becomes a `RefCell`.

```rust
type Elements = HashMap<WindowId, Watched>;

struct Watched {
    element: Element,
    frame: Frame,
}

struct WatcherState {
    /// Every window being watched. A `RefCell` and not a `Mutex`: nothing off the main thread
    /// reaches it.
    elements: RefCell<Elements>,
    apps: RefCell<HashMap<Pid, AppObserver>>,
    on_change: Box<dyn Fn(WindowChange)>,
}

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
    /// observed cannot be reported here, because the lookup happens after the send;
    /// [`Watcher::pump`] logs it at `debug` instead.
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        self.placements
            .send(target)
            .map_err(|_| WindowError::NotWatching)
    }
}
```

`WindowError::UnknownWindow` loses its last producer and goes, with its `Display` arm:

```rust
pub enum WindowError {
    /// [`watch`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// The watcher has been dropped, so nothing is being observed at all.
    NotWatching,
}
```

### `Watcher` and `pump`

```rust
pub struct Watcher {
    /// The workspace and screen observations. Held for their `Drop`, and declared first so they
    /// stop before the state they write into is torn down: fields drop in declaration order.
    _notifications: Vec<Observation>,
    /// Handed to every [`WindowSink`].
    placements_sender: WakingSender<WindowFrame>,
    /// Placements waiting to be performed. Drained by [`Self::pump`] on the main thread.
    placements: Receiver<WindowFrame>,
    state: Rc<WatcherState>,
}

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
            let found = self
                .state
                .elements
                .borrow()
                .get(&target.window)
                .map(|watched| (watched.element.retained(), watched.frame));
            let Some((element, from)) = found else {
                tracing::debug!(?target, "no such window to place");
                continue;
            };
            std::thread::spawn(move || {
                set_frame(element.raw(), from, target.frame);
                tracing::debug!(?target, ?from, "set a window's frame");
            });
        }
    }
}
```

The `borrow` is bound to `found` and ends before the spawn, so a `Drop` running on the main thread cannot re-enter it.

`Element::retained` replaces what `Arc::clone` did:

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

returning

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

### Every table access loses its lock

```rust
    fn forget(&self, window: WindowId) -> bool {
        self.elements.borrow_mut().remove(&window).is_some()
    }

    fn record(&self, window: WindowId, frame: Frame) {
        if let Some(watched) = self.elements.borrow_mut().get_mut(&window) {
            watched.frame = frame;
        }
    }

    fn forget_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        let mut table = self.elements.borrow_mut();
        // SAFETY: both are live `AXUIElement`s as far as CoreFoundation is concerned. A destroyed
        // element is still a valid CF object; it is the Accessibility calls on it that fail.
        #[expect(unsafe_code)]
        let found = table
            .iter()
            .find(|(_, held)| unsafe { CFEqual(held.element.raw().cast(), element.cast()) != 0 })
            .map(|(id, _)| *id)?;
        table.remove(&found);
        Some(found)
    }
```

`observe_window`'s insert:

```rust
    state.elements.borrow_mut().insert(
        window,
        Watched {
            element: Element(owned),
            frame,
        },
    );
```

`forget_app`, where the `borrow` must end before the loop that calls `forget` and `report`, or it is a `BorrowMutError`:

```rust
    let gone: Vec<WindowId> = state
        .elements
        .borrow()
        .iter()
        .filter(|(_, watched)| window_id(watched.element.raw()).is_none())
        .map(|(id, _)| *id)
        .collect();
    for window in gone {
        if state.forget(window) {
            state.report(WindowChange::Closed(window));
        }
    }
```

That strictness fails loudly on the first run rather than quietly in production, which the `Mutex` did not offer.

The snapshot:

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

### Imports

`use std::sync::{Arc, Mutex, Weak};` goes. `freddie_main_loop::{MainWaker, WakingSender}` and `std::sync::mpsc::Receiver` come in. `std::cell::RefCell` and `core_foundation::base::CFRetain` are already imported. `unsafe impl Sync for Owned` is deleted; `unsafe impl Send for Owned` stays, with its comment changed to say the element is moved to the placement thread rather than shared through a lock.

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

## Call sites

Changes 0 through 4 touch only `crates/freddie_windows/src/lib.rs`, apart from Change 0 deleting a variant nothing matches on. Change 5 changes `watch`'s signature and adds `Watcher::pump`, so mercury's two edits are in that commit and figaro needs the same two.

`WindowChange`, `Snapshot`, `WindowFrame` and `WindowId` keep their shapes throughout. `WindowSink::set_frame` keeps its signature and changes only when it returns.

## Verification

```
cargo test -p freddie_windows -p mercury
cargo clippy --all-targets --all-features
```

`writes_for` is covered by the table in Change 4. The rest needs a window server and the Accessibility grant, so by hand after `mercury restart`:

1. Place a window that grows and moves left, then one that shrinks and moves right, from the resize layer. Both land exactly on the target rather than short of it.
2. `mercury logs --level debug` shows no `an attribute write was refused` lines on placements that used to need the second pass to converge.
3. A placement is measurably shorter. Time from the `SetFrame` dispatch record to `set a window's frame` drops by roughly a quarter to a half, being one or two IPC round trips saved out of four.
5. After Change 5, placement still does not stall the keyboard: hold a key while placing and keys keep emitting at the usual rate.
6. After Change 5, place a window that has just closed: the log shows `no such window to place` and nothing else happens.
7. Window changes still arrive at all, which is what confirms the table is written correctly through its new `RefCell`: open and close a window and see `Opened` and `Closed`.

## Ordered commits

1. Change 0: `WindowError` drops `NoFocusedWindow`, which nothing produces.
2. Change 1: `set_attribute` warns on a refused write.
3. Change 2: `Watched` holds the element and its last reported frame, `record` replaces it on move and resize, and the snapshot reads it rather than the OS.
4. Change 3: `report_open` is handed the frame `observe_window` recorded instead of reading it again.
5. Change 4: `writes_for` decides the order, `set_frame` performs shrink, move, grow, and the tests table it.
6. Change 5: `WindowSink` becomes a sender, the table becomes a main-thread `RefCell`, `Watcher::pump` performs placements, `Arc`, `Mutex`, `Weak` and `UnknownWindow` leave, and mercury pumps and drops its helper.
