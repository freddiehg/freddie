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

# Change 2: the overlay is a handle, not a thread-local

`freddie_overlay` keeps its panel in a `thread_local!` and exposes free `show` and `hide`. `freddie_menu_bar` next door does the same job — a main-thread-only AppKit object with a lifetime — through a handle it returns.

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
/// The overlay panel. Holding it keeps the panel available; dropping it takes it down.
///
/// `!Send`, like `freddie_menu_bar`'s `MenuBar`: an `NSPanel` belongs to the main thread
/// that built it.
pub struct Overlay {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
}

/// Build the overlay panel, hidden. Main thread only.
///
/// # Panics
///
/// If called off the main thread, where `NSPanel` cannot be built.
#[must_use]
pub fn overlay() -> Overlay { ... }

impl Overlay {
    /// Show `text`, sizing the panel to it. Main thread only.
    pub fn show(&self, text: &str) { ... }

    /// Take the overlay down. A no-op if it is not up.
    pub fn hide(&self) { ... }
}
```

`text` stops being `&'static str`. The bound existed because the dispatched block had to be `'static`; a method on a handle the caller already holds borrows it for the call instead.

Building it eagerly rather than on first `show` is what removes the `Option`: an `Overlay` that exists has a panel.

## What the caller does instead

`Overlay` is `!Send` and its methods are main-thread-only, so the effect loop cannot call them directly. It sends, and the main thread applies — the same shape the menu-bar title already uses, and the same channel drained in the same place.

`crates/mercury/src/daemon.rs`, before:

```rust
        MercuryEffect::ShowOverlay(text) => freddie_overlay::show(text),
        MercuryEffect::HideOverlay => freddie_overlay::hide(),
```

After, sending on a channel built beside `title_tx`:

```rust
        MercuryEffect::ShowOverlay(text) => {
            let _ = overlay_tx.send(Some(text));
        }
        MercuryEffect::HideOverlay => {
            let _ = overlay_tx.send(None);
        }
```

and drained in `main_loop.run`, beside the title:

```rust
    main_loop.run(|| {
        if let Some(showing) = overlay_rx.try_iter().last() {
            match showing {
                Some(text) => overlay.show(text),
                None => overlay.hide(),
            }
        }
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
    });
```

Only the last showing in a batch is drawn, for the reason the title already gives: intermediate states in one batch are not worth putting on screen.

This costs the overlay up to one `SLICE` of latency, where the dispatch to the main queue had none. `freddie_main_loop`'s `SLICE` is 100ms. If that shows, the answer is the `Waker` described in the note at the end of this doc, which the menu-bar title wants for the same reason.

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

---

## A note on waking the main loop

Change 2 puts the overlay behind the same channel the menu-bar title uses, and both then wait up to one `SLICE` (100ms) for `on_wake` to run.

`freddie_main_loop` should hand back a `Waker` alongside its `Stopper`, so a sender can make `on_wake` run now:

```rust
/// Wakes the main loop so its `on_wake` runs now rather than when the current slice
/// expires. `Send`, so a worker can hold one.
pub struct Waker { ... }

impl Waker {
    pub fn wake(&self) { ... }
}
```

`CFRunLoop::wake_up` is not established to do it: the main thread is inside
`nextEventMatchingMask_untilDate_inMode_dequeue`, which returns when an event is available
or its deadline passes, and a bare wake makes neither true. Posting an application-defined
`NSEvent` with `postEvent_atStart` makes an event available, which is the mechanism to
verify first.

It does not cover menu tracking: while a menu is open AppKit runs a modal loop and the outer
pump does not iterate, so `on_wake` waits for the menu to close whatever is posted.

This is worth doing only if the latency shows. It is written down here because Change 2 is
what makes a second caller want it.
