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

So the storage is not the problem; being able to reach it without proving it exists is. It becomes private, and a handle only [`overlay`] can mint is what reaches it.

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
    /// The panel and its label, on the main thread, reachable from a dispatched block.
    ///
    /// Private, and only an [`Overlay`] can reach it: a block dispatched to the main queue
    /// has to be `'static` and `Send`, so it cannot carry an `NSPanel` and finds one here
    /// instead. `None` before [`overlay`] builds it and after the handle drops.
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

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
/// `!Send`, because `Drop` reaches `PANEL`, and a `thread_local` reached from another
/// thread is a different slot: a handle dropped off main would clear an empty one and
/// leave the real panel on screen. It stays where [`overlay`] built it, like
/// `freddie_menu_bar`'s `MenuBar` and `freddie_windows`'s `Watcher`.
///
/// It does not show anything. [`sink`](Overlay::sink) is what a worker uses.
pub struct Overlay {
    _main_thread_only: PhantomData<*const ()>,
}

/// The handle showing and hiding go through. Cheap to clone and `Send`, because it carries
/// nothing: `show` and `hide` dispatch to the main queue and find the panel there.
///
/// Safe to keep past the [`Overlay`]. The dispatched block finds an empty slot and does
/// nothing, which is what a hidden overlay would have done anyway.
#[derive(Clone, Copy)]
pub struct OverlaySink;

/// Build the overlay panel, hidden, and return the handle that owns it.
///
/// Eagerly, not on first show: it is what makes the slot `Some` for the whole life of the
/// [`Overlay`], so nothing after this has to write it and `show` can borrow it shared.
///
/// # Panics
///
/// If called off the main thread, where `NSPanel` cannot be built.
#[must_use]
pub fn overlay() -> Overlay {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    PANEL.with_borrow_mut(|slot| *slot = Some(build(mtm)));
    Overlay {
        _main_thread_only: PhantomData,
    }
}

impl Overlay {
    /// A handle to show and hide through. Cheap to clone, `Send`, and safe to keep past
    /// the overlay itself.
    #[must_use]
    pub const fn sink(&self) -> OverlaySink {
        OverlaySink
    }
}

impl OverlaySink {
    /// Show the overlay with `text`, from any thread.
    ///
    /// The panel is sized to the text, so a keymap with more rows makes a taller panel
    /// rather than a clipped one.
    pub fn show(&self, text: String) {
        DispatchQueue::main().exec_async(move || {
            // Shared, not mutable: every AppKit setter here takes `&self`, and the slot
            // itself is only written by `overlay` and `Overlay::drop`.
            PANEL.with_borrow(|slot| {
                let Some(panel) = slot else { return };
                // ... set the label, resize, place, order front, as today
            });
            debug!(text, "overlay shown");
        });
    }

    /// Hide the overlay, from any thread. A no-op if it is not up.
    pub fn hide(&self) {
        DispatchQueue::main().exec_async(|| {
            PANEL.with_borrow(|slot| {
                if let Some(panel) = slot {
                    panel.panel.orderOut(None);
                }
            });
            debug!("overlay hidden");
        });
    }
}

impl Drop for Overlay {
    /// Takes the panel off screen and gives it back.
    ///
    /// Dropping the `Retained` alone does neither: AppKit's window list holds its own
    /// reference to a window, so the panel would stay alive, and stay visible, with nothing
    /// on this side able to reach it.
    ///
    /// `close` is what drops AppKit's reference. It is safe to call because `build` cleared
    /// `releasedWhenClosed`, so closing does not also release — the `Retained` going out of
    /// scope here is the last reference, and the panel is deallocated.
    fn drop(&mut self) {
        PANEL.with_borrow_mut(|slot| {
            if let Some(panel) = slot.take() {
                panel.panel.orderOut(None);
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
