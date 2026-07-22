# Platform crates own what they register

Three macOS crates hold process-global state or leave a registration behind when their handle drops. Each gets the shape `freddie_app_nav` and `freddie_windows` already have: a handle that owns what it registered and undoes it on `Drop`.

The rule they end up following is the one those two already do. Registering with the OS returns something; that something belongs to a value the caller holds; dropping it deregisters. Nothing is reachable through a static, and nothing outlives the handle that created it.

---

# Change 1: `MenuBar` deregisters its menu handler

`MenuEvent::set_event_handler` is process-global in muda, and `MenuBar` has no `Drop`. Dropping one takes the icon down and leaves the handler installed, holding `on_quit` alive for the life of the process. Calling `show` twice replaces the first handler, so the first icon stays visible with a Quit that no longer fires.

Mercury is single-instance and calls `show` once, so nothing hits this today. The crate is written as though you could call it twice.

`crates/freddie_menu_bar/src/lib.rs`, before:

```rust
/// A live status item. Holding it keeps the icon up; dropping it takes the icon down.
pub struct MenuBar {
    tray: TrayIcon,
}
```

After:

```rust
/// A live status item. Holding it keeps the icon up; dropping it takes the icon down and
/// deregisters the menu handler.
pub struct MenuBar {
    tray: TrayIcon,
}

impl Drop for MenuBar {
    /// Clears the global menu handler this `MenuBar` installed.
    ///
    /// `MenuEvent::set_event_handler` is one handler for the process, so leaving it
    /// installed would keep `on_quit` alive after the icon is gone, and a second `show`
    /// would silently replace the first one's.
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    }
}
```

The doc comment on [`show`] gains the constraint that follows from a global handler:

```rust
/// One at a time: the menu handler is process-global, so a second `MenuBar` replaces the
/// first one's, and dropping either clears it. Build one and hold it.
```

# Change 2: the overlay's storage is reachable only through a handle

`freddie_overlay` exposes free `show` and `hide`, so calling them before anything built the panel is representable, and nothing says when the panel goes away. `freddie_menu_bar` next door answers both with a handle it returns.

The `thread_local!` stays. It is what lets a block dispatched to the main queue reach the panel: the closure must be `'static` and `Send`, so it cannot carry an `NSPanel`, and it finds one already on main instead. That dispatch is also what makes the overlay appear immediately — the main queue is drained from inside `nextEventMatchingMask_untilDate`, without waiting for it to return.

So the storage is not the problem; being able to reach it without proving it exists is. It becomes private, keyed by overlay rather than a single slot, and a handle only [`overlay`] can mint is what reaches an entry.

Two things follow from that and are worth stating as requirements rather than as things that happen to work. Overlays are not a singleton: [`overlay`] can be called more than once, each handle drives its own panel, and one dropping leaves the others alone. And an overlay is destroyable: dropping one deallocates its panel rather than hiding it, ids are never reused, and an [`OverlaySink`] outliving its overlay is inert rather than pointed at somebody else's.

## What hiding is for, and what dropping is for

An overlay's lifetime is the lifetime of whatever state holds it, and those differ.

The keymap overlay is held for the run. It is shown and hidden on every layer change, so its panel stays built between showings: `hide` orders it out and keeps it, and the next keystroke puts an existing panel on screen rather than constructing one. That is the whole reason `hide` is not `drop`.

An overlay shown for an infrequent event, and only briefly, is the other case. Its state appears, an [`Overlay`] is built with it, and when that state goes the handle drops. Nothing keeps a panel warm for something that may not happen again, and the drop has to give the panel back rather than park it hidden and alive. `close` is what does that, and `build` clears `releasedWhenClosed` so the release is ours to perform.

So `hide` is for an overlay that will be shown again, and `drop` is for one that will not.

`crates/freddie_overlay/src/lib.rs`, before:

```rust
thread_local! {
    // The panel and its label, on the main thread. Only ever touched inside a dispatched
    // block, which always runs on main.
    static PANEL: RefCell<Option<(Retained<NSPanel>, Retained<NSTextField>)>> =
        const { RefCell::new(None) };
}

pub fn show(text: &'static str) { ... }
pub fn hide() { ... }
```

After:

```rust
thread_local! {
    /// Every overlay built on this thread, by id.
    ///
    /// Private, and only an [`Overlay`] or [`OverlaySink`] can reach an entry: a block
    /// dispatched to the main queue has to be `'static` and `Send`, so it cannot carry an
    /// `NSPanel` and looks one up here instead. An id is in the table between [`overlay`]
    /// building it and the handle dropping.
    ///
    /// A table and not a single slot: nothing about a panel makes it the only one, and a
    /// consumer wanting two overlays should not have to fork the crate to get them.
    static PANELS: RefCell<HashMap<OverlayId, Panel>> = RefCell::new(HashMap::new());

    /// Mints the next [`OverlayId`]. A plain `Cell` because overlays are only ever built on
    /// this thread.
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

/// One overlay's entry in [`PANELS`]. Ids are never reused within a run.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct OverlayId(u64);

struct Panel {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
}

// In `build`, once the panel exists and before it is ever ordered in:
//
// SAFETY: setting a window's own release policy, on the main thread, before anything else
// holds it.
#[expect(unsafe_code)]
unsafe {
    // Ours to release, not AppKit's. `NSWindow` defaults to releasing itself when closed,
    // which would leave `Overlay::drop`'s `close` racing the `Retained` it still holds.
    // Cleared here so `close` only drops AppKit's reference and ours is the last one.
    panel.setReleasedWhenClosed(false);
}

/// The overlay's lifetime. Holding it keeps the panel built; dropping it takes it down.
///
/// `!Send`, because `Drop` reaches `PANELS`, and a `thread_local` reached from another
/// thread is a different table: a handle dropped off main would find no entry and leave
/// the real panel on screen. It stays where [`overlay`] built it, like
/// `freddie_menu_bar`'s `MenuBar` and `freddie_windows`'s `Watcher`.
///
/// It does not show anything. [`sink`](Overlay::sink) is what a worker uses.
pub struct Overlay {
    id: OverlayId,
    _main_thread_only: PhantomData<*const ()>,
}

/// The handle showing and hiding go through. Cheap to clone and `Send`, because it carries
/// nothing: `show` and `hide` dispatch to the main queue and find the panel there.
///
/// Safe to keep past its [`Overlay`]. The dispatched block finds no entry for its id and
/// does nothing, which is what hiding an already-hidden overlay would have done. Ids are
/// never reused, so a later overlay cannot inherit a dead sink's messages.
#[derive(Clone, Copy)]
pub struct OverlaySink {
    id: OverlayId,
}

/// Build the overlay panel, hidden, and return the handle that owns it.
///
/// Eagerly, not on first show: it is what keeps the entry present for the whole life of the
/// [`Overlay`], so nothing after this has to write the table and `show` can borrow it
/// shared.
///
/// # Panics
///
/// If called off the main thread, where `NSPanel` cannot be built.
#[must_use]
pub fn overlay() -> Overlay {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        OverlayId(id)
    });
    PANELS.with_borrow_mut(|panels| panels.insert(id, build(mtm)));
    Overlay {
        id,
        _main_thread_only: PhantomData,
    }
}

impl Overlay {
    /// A handle to show and hide through. Cheap to clone, `Send`, and safe to keep past
    /// the overlay itself.
    #[must_use]
    pub const fn sink(&self) -> OverlaySink {
        OverlaySink { id: self.id }
    }
}

impl OverlaySink {
    /// Show the overlay with `text`, from any thread.
    ///
    /// The panel is sized to the text, so a keymap with more rows makes a taller panel
    /// rather than a clipped one.
    pub fn show(&self, text: String) {
        let id = self.id;
        DispatchQueue::main().exec_async(move || {
            // Shared, not mutable: every AppKit setter here takes `&self`, and the table
            // itself is only written by `overlay` and `Overlay::drop`.
            PANELS.with_borrow(|panels| {
                let Some(panel) = panels.get(&id) else { return };
                // ... set the label, resize, place, order front, as today
            });
            debug!(text, "overlay shown");
        });
    }

    /// Hide the overlay, from any thread. A no-op if it is not up.
    pub fn hide(&self) {
        let id = self.id;
        DispatchQueue::main().exec_async(move || {
            PANELS.with_borrow(|panels| {
                if let Some(panel) = panels.get(&id) {
                    panel.panel.orderOut(None);
                }
            });
            debug!("overlay hidden");
        });
    }
}

impl Drop for Overlay {
    /// Gives the panel back.
    ///
    /// Dropping the `Retained` alone would not: AppKit's window list holds its own
    /// reference to a window, so the panel would stay alive, and stay on screen, with
    /// nothing on this side able to reach it.
    ///
    /// `close` takes it off the screen and off that list, so no `orderOut` is needed first.
    /// It is safe to call because `build` cleared `releasedWhenClosed`, so closing does not
    /// also release — the `Retained` going out of scope here is the last reference, and the
    /// panel is deallocated.
    fn drop(&mut self) {
        PANELS.with_borrow_mut(|panels| {
            if let Some(panel) = panels.remove(&self.id) {
                panel.panel.close();
            }
        });
    }
}
```

`text` becomes an owned `String` rather than `&'static str`. The `'static` bound was never about the panel; it was the dispatched block needing to own what it carries, and a `String` it owns satisfies that without every caller having to hand it a const.

Mercury builds the `Overlay` on the main thread beside the `MenuBar`, holds it for the life
of `main`, and hands `overlay.sink()` to the effect loop the way it already hands it a
`WindowSink`:

```rust
        MercuryEffect::ShowOverlay(text) => overlay.show(text.to_owned()),
        MercuryEffect::HideOverlay => overlay.hide(),
```

where `overlay` is the `OverlaySink`. Nothing goes through `main_loop.run`, and the overlay
keeps appearing as promptly as it does today.

# Change 3: dropping an `Interceptor` cannot hang

`Interceptor::drop` stops the tap thread's run loop and then joins it, with no bound. A tap thread wedged inside `on_key` makes the drop block forever — during shutdown, and during a panic unwind, where a second panic would abort.

`on_key` is mercury's dispatch and is fast, so this is a hazard rather than a live bug.

`crates/freddie_keyboard/src/sys/macos.rs`, before:

```rust
impl Drop for Interceptor {
    fn drop(&mut self) {
        self.run_loop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
```

After:

```rust
/// How long a dropped `Interceptor` waits for the tap thread to finish before giving up on
/// it.
///
/// Stopping the run loop is what ends that thread, and it ends promptly unless it is inside
/// a slow `on_key`. Waiting forever would turn one wedged callback into a process that
/// cannot exit, and this runs on the shutdown path and during unwinds.
const RELEASE_TIMEOUT: Duration = Duration::from_millis(500);

impl Drop for Interceptor {
    fn drop(&mut self) {
        self.run_loop.stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        // Joined on another thread so this one can stop waiting. The tap is released when
        // the thread ends either way; what the timeout bounds is how long the caller waits
        // to hear about it.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread.join();
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(RELEASE_TIMEOUT).is_err() {
            tracing::warn!("the keyboard tap did not stop; releasing without it");
        }
    }
}
```

The keyboard comes back regardless: the tap dies with the process even if the thread never unwinds.
