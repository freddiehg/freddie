# a placement is a message, not a shared table

`freddie_windows` is the only crate in the family that shares mutable state across threads. The element table is an `Arc<Mutex<HashMap<..>>>`, a `WindowSink` holds a `Weak` to it, and the values are `Arc<Element>` so one can be cloned out from under the lock:

```rust
struct Elements(Mutex<HashMap<WindowId, Arc<Element>>>);

pub struct WindowSink {
    elements: Weak<Elements>,
}
```

Every one of those three exists because the table is reachable from two threads. The AX callbacks write it on the main thread; `WindowSink::set_frame` reads it from whatever thread the caller is on. Nothing else about the crate needs them: `WatcherState::apps` is a bare `RefCell` because it is main-thread only, and the table is the odd one out.

A channel carries the placement instead. The table stops leaving the main thread, and `Arc`, `Mutex` and `Weak` all lose their reason at once.

The reason the table was shared rather than owned is that neither end may block. The table belongs to the main thread because that is where the observers deliver. The AX write must not run there, because main is a serialized doorman and a write costing tens of milliseconds would stall every other source. A channel satisfies both: main owns the table and does the lookup, which is a hashmap hit, and hands the element it found to a thread of its own for the write.

## Shape after

The sink is a sender. The table is a plain field beside `apps`. The main thread drains placements on each wake, exactly as `freddie_overlay` already does with `Overlay::pump`.

```rust
/// Every window that can be addressed, and the element to address it through.
///
/// Main-thread only, like `apps`: the AX callbacks that write it and the `pump` that reads it
/// both run there, so there is nothing to lock.
type Elements = HashMap<WindowId, Element>;

/// What the [`Watcher`] holds, reachable from the callbacks as well as from it.
struct WatcherState {
    /// Every window being watched. A `RefCell` and not a `Mutex`: nothing off the main
    /// thread reaches it.
    elements: RefCell<Elements>,
    /// One entry per observed app. Held here rather than on the [`Watcher`] because the
    /// launch and terminate callbacks are `'static` closures that cannot borrow it.
    apps: RefCell<HashMap<Pid, AppObserver>>,
    on_change: Box<dyn Fn(WindowChange)>,
}

/// The handle a placement is performed through.
///
/// Cheap to clone and unattached to the thread that made it: it is a sender, and the
/// placement is looked up and performed by the thread that owns the table.
///
/// Dropping the [`Watcher`] drops the receiver, so a send from a sink that has outlived its
/// watcher is how [`WindowError::NotWatching`] is answered.
#[derive(Clone)]
pub struct WindowSink {
    placements: WakingSender<WindowFrame>,
}

/// Holds every registration that makes windows report, and the placement queue.
pub struct Watcher {
    /// The workspace and screen observations. Held for their `Drop`, and declared first so
    /// they stop before the state they write into is torn down: fields drop in declaration
    /// order.
    _notifications: Vec<Observation>,
    /// Placements waiting to be performed. Drained by [`Watcher::pump`] on the main thread.
    placements: Receiver<WindowFrame>,
    state: Rc<WatcherState>,
}
```

`Element` is stored by value rather than behind an `Arc`, because nothing clones one out any more. The `unsafe impl Send for Owned` stays and its reason changes: the element still crosses to the thread doing the write, but now as an explicit move rather than as access to a shared table. `unsafe impl Sync for Owned` goes, because nothing shares one.

## Change 1: the table stops being shared

File: `crates/freddie_windows/src/lib.rs`.

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
        // Cloned out so the lock is released before the writes: those take tens of
        // milliseconds, and the main thread takes this lock every time a window opens or
        // closes.
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
    /// itself costs tens of milliseconds and runs on a thread of its own, so this returns
    /// immediately and a caller on a latency-sensitive loop needs no thread of its own.
    ///
    /// The frame is the caller's, already worked out. This does not consult the screen, the
    /// frontmost app, or anything else.
    ///
    /// # Errors
    ///
    /// [`WindowError::NotWatching`] if the watcher has been dropped. A window that is not being
    /// observed cannot be reported here, because the lookup happens after the send; `pump` logs
    /// it at `debug` instead.
    pub fn set_frame(&self, target: WindowFrame) -> Result<(), WindowError> {
        self.placements
            .send(target)
            .map_err(|_| WindowError::NotWatching)
    }
}
```

### `Watcher::pump`

New, and the counterpart to `Overlay::pump`. Called from the main loop's `on_wake`.

```rust
impl Watcher {
    /// A handle to perform placements through. Cheap to clone, `Send`, and safe to keep past
    /// the watcher, which it answers [`WindowError::NotWatching`] from.
    #[must_use]
    pub fn sink(&self) -> WindowSink {
        WindowSink {
            placements: self.placements_sender.clone(),
        }
    }

    /// Perform every placement queued since the last wake.
    ///
    /// On the main thread, because that is where the element table lives. The lookup is a
    /// hashmap hit; the write is handed to a thread of its own, because it costs tens of
    /// milliseconds and this thread is what every other source is waiting on.
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

`Watcher` needs the sender to hand out from `sink`, so it holds both halves:

```rust
pub struct Watcher {
    _notifications: Vec<Observation>,
    placements_sender: WakingSender<WindowFrame>,
    placements: Receiver<WindowFrame>,
    state: Rc<WatcherState>,
}
```

`Element::retained` is what makes the spawned thread own its element rather than borrow the table:

```rust
impl Element {
    /// A second owned reference to the same element, for handing to another thread.
    ///
    /// `CFRetain` rather than a clone of the `Owned`, which is deliberately not `Clone`
    /// because two of those naming one reference would release it twice.
    fn retained(&self) -> Self {
        // SAFETY: `self` holds a live +1 reference, so retaining it yields a second one, which
        // the returned `Element` releases on drop.
        #[expect(unsafe_code)]
        let raw = unsafe { CFRetain(self.raw().cast()) };
        Self(Owned(raw))
    }
}
```

### `watch`

Takes the waker, so it can make the channel. Everything else about its body is unchanged except that the table is a `RefCell` and the snapshot reads it with `borrow` instead of `lock`.

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

and the snapshot's read:

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

with the `Watcher` built as

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

Each one loses its lock and its `Arc`. `WatcherState`'s two removers:

```rust
    /// Stop being able to address `window`. Whether there was an entry to remove.
    fn forget(&self, window: WindowId) -> bool {
        self.elements.borrow_mut().remove(&window).is_some()
    }

    /// Forget whichever window `element` names, and say which it was.
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

`forget_app`'s scan:

```rust
    let gone: Vec<WindowId> = state
        .elements
        .borrow()
        .iter()
        .filter(|(_, element)| window_id(element.raw()).is_none())
        .map(|(id, _)| *id)
        .collect();
```

The `borrow` there ends before the loop that calls `forget` and `report`, which would otherwise be a `BorrowMutError` rather than a deadlock. That is the one way this is stricter than the `Mutex` it replaces, and it is stricter in the direction that fails loudly at the first test rather than quietly in production.

### Imports

`use std::sync::{Arc, Mutex, Weak};` goes. `std::cell::RefCell` is already imported. `freddie_main_loop::{MainWaker, WakingSender}` and `std::sync::mpsc::Receiver` come in, and `core_foundation::base::CFRetain` is already imported.

`unsafe impl Sync for Owned` is deleted; `unsafe impl Send for Owned` stays with its comment rewritten to say the element is moved to the placement thread rather than shared through a lock.

## Change 2: mercury stops spawning for placements

File: `crates/mercury/src/daemon.rs`. Depends on Change 1.

The watch call takes the waker, which `run` already has:

```rust
    let windows = freddie_windows::watch(&waker, {
        let event_tx = event_tx.clone();
        move |change| {
            let _ = event_tx.send(MercuryEvent::Window(WindowEvent { change }));
        }
    });
```

The main loop drains placements beside the overlay:

```rust
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
        overlay.pump();
        if let Some(watcher) = _window_watcher.as_ref() {
            watcher.pump();
        }
    });
```

which means `_window_watcher` is read rather than only held, so it is renamed `window_watcher`.

The effect loses its helper. Before:

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

After, the whole helper is deleted and the arm queues directly, because `set_frame` no longer blocks:

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

`freddie_windows`'s public surface changes in two places: `watch` takes a `&MainWaker` first, and `Watcher` gains `pump`. `WindowSink`, `WindowChange`, `Snapshot`, `WindowFrame` and `WindowId` are unchanged, and `set_frame` keeps its signature while changing when it returns.

Mercury is the only consumer in this repository. figaro takes `freddie_windows` by path and will need the same two edits, which are the ones in Change 2.

`WindowError::UnknownWindow` keeps its variant and loses this producer; `focused_window_id` and the seed path still use the rest of the enum.

## Verification

```
cargo test -p freddie_windows -p mercury
cargo clippy --all-targets --all-features
```

The crate's tests are `Frame` geometry and are untouched. What this changes cannot be unit tested without a window server and the Accessibility grant, so the check is by hand after `mercury restart`:

1. Place a window: enter the resize layer and press an arrow. The window moves, and the log shows `set a window's frame` from the placement thread.
2. Place a window that has just closed, which is the path that used to answer `UnknownWindow`: the log shows `no such window to place` at `debug` and nothing else happens.
3. Placement does not stall the keyboard. Hold a key while placing: keys keep emitting at the usual rate, because the AX write is still off the main thread and off the effect loop.
4. The daemon still reports window changes at all, which is what confirms the table is being written through its new `RefCell`: open and close a window and see `Opened` and `Closed`.

## Ordered commits

1. Change 1: `Elements` becomes a main-thread `RefCell`, `WindowSink` becomes a sender, `Watcher::pump` performs placements, `Arc`, `Mutex` and `Weak` leave the crate.
2. Change 2: mercury passes the waker to `watch`, pumps the watcher on each wake, and drops its `set_frame` helper.
