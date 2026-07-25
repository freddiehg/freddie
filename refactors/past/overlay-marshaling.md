# overlay: a channel to the main thread, not a thread-local table

`freddie_overlay` shows and hides an `NSPanel` from any thread. Today the marshaling to the main thread goes through libdispatch, and because a block dispatched to the main queue must be `'static + Send`, it cannot carry the `NSPanel`. So the panels live in a `thread_local!` table and the dispatched block looks one up by id:

```rust
thread_local! {
    static PANELS: RefCell<HashMap<OverlayId, Panel>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

pub fn show(&self, text: String) {
    let id = self.id;
    DispatchQueue::main().exec_async(move || {
        PANELS.with_borrow(|panels| {
            let Some(Panel { panel, label }) = panels.get(&id) else { return };
            // ... mutate the panel ...
        });
    });
}
```

The table, the id it is keyed by, and the `Cell` that mints the id are all there to route a `Send` block back to a non-`Send` panel. `freddie_main_loop::MainLoop::run` already gives the main thread an `on_wake` callback for exactly this kind of work, and `daemon.rs` already drains the menu-bar title channel there. Sending over a channel drained on `on_wake` lets the `Overlay` own its panel directly, which deletes the `thread_local!`, the id, and the table.

The channel is a `WakingSender`, so a `show` wakes the main run loop and `pump` runs at once — the promptness GCD gave for free. This change depends on `refactors/past/wake-the-main-loop.md`, which has already landed.

## The shape

Each `Overlay` owns its `Panel` and the receiving end of a channel. `OverlaySink` holds the sending end, which is `Send` and `Clone`. `show`/`hide` send a message; the main thread drains it and mutates the panel it owns. There is no shared table, so there is no id and no lookup. Drain every queued message in order (not `.last()` the way the title does): a `Hide` after a `Show` in the same wake must still hide.

### Cargo

`crates/freddie_overlay/Cargo.toml`. before:

```toml
[dependencies]
dispatch2 = "0.3"
objc2 = "0.6"
# ...
```

after:

```toml
[dependencies]
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
objc2 = "0.6"
# ... (no dispatch2)
```

### Imports and crate docs

`crates/freddie_overlay/src/lib.rs`. Module docs, before:

```rust
//! [`overlay`] builds one on the main thread and returns the [`Overlay`] that owns it. Dropping
//! that closes the panel and gives it back. [`Overlay::sink`] hands out an [`OverlaySink`], which
//! is `Send`: [`OverlaySink::show`] and [`OverlaySink::hide`] are callable from any thread and
//! marshal to the main thread, where `AppKit` lives, by dispatching onto the main queue. It is
//! serviced by the main run loop, so this needs `freddie_main_loop` running and `NSApp`
//! initialized, the same as the menu bar.
```

after:

```rust
//! [`overlay`] builds one on the main thread and returns the [`Overlay`] that owns the panel
//! beside the first [`OverlaySink`]. Dropping the overlay closes the panel. The sink is `Send`
//! and `Clone`: [`OverlaySink::show`] and [`OverlaySink::hide`] are callable from any thread and
//! send over a [`freddie_main_loop::WakingSender`], which wakes the main run loop so
//! [`Overlay::pump`] (called from `on_wake`) applies the change. Needs `freddie_main_loop`
//! running and `NSApp` initialized, the same as the menu bar.
```

Imports, before:

```rust
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
// ...
```

after:

```rust
use std::sync::mpsc::Receiver;

use freddie_main_loop::{MainWaker, WakingSender};
use objc2::MainThreadMarker;
// ...
```

### Types

A message type replaces the table and id:

```rust
/// What a sink asks its overlay to do. Sent over the channel, drained on the main thread.
enum OverlayMsg {
    /// Show with this text, sizing the panel to it.
    Show(String),
    /// Take the panel off the screen; the panel stays built.
    Hide,
}
```

`Overlay`, before:

```rust
pub struct Overlay {
    id: OverlayId,
    _main_thread_only: PhantomData<*const ()>,
}
```

after:

```rust
pub struct Overlay {
    /// The panel this overlay owns. `Retained<NSPanel>` is not `Send`, which keeps `Overlay` on
    /// the thread that built it without a `PhantomData`.
    panel: Panel,
    /// Drained by [`Overlay::pump`] on the main thread when the loop wakes. The overlay holds only
    /// this end of the channel; the sinks hold the senders.
    message_receiver: Receiver<OverlayMsg>,
}
```

`OverlaySink`, before:

```rust
#[derive(Clone, Copy, Debug)]
pub struct OverlaySink {
    id: OverlayId,
}
```

after (`WakingSender` has no `Debug`, so hand-write a non-exhaustive one rather than deriving or adding `Debug` up the main-loop stack):

```rust
/// The handle showing and hiding go through. `Send` and `Clone`, so any thread can hold one; the
/// panel it drives is on the main thread, reached by sending rather than by touching it.
///
/// Safe to keep past its [`Overlay`]: once the overlay is dropped the receiver is gone, and a send
/// is a harmless error, which is what hiding an already-gone overlay would have been.
#[derive(Clone)]
pub struct OverlaySink {
    message_sender: WakingSender<OverlayMsg>,
}

impl std::fmt::Debug for OverlaySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlaySink").finish_non_exhaustive()
    }
}
```

### Construction

`overlay()` takes the main-loop waker, builds the panel and a waking channel, and hands back the panel-owning `Overlay` beside the first `OverlaySink`. before:

```rust
pub fn overlay() -> Overlay {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        OverlayId(id)
    });
    PANELS.with_borrow_mut(|panels| panels.insert(id, build(mtm)));
    debug!(?id, "overlay built");
    Overlay {
        id,
        _main_thread_only: PhantomData,
    }
}

impl Overlay {
    pub const fn sink(&self) -> OverlaySink {
        OverlaySink { id: self.id }
    }
}
```

after:

```rust
pub fn overlay(waker: &MainWaker) -> (Overlay, OverlaySink) {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let (message_sender, message_receiver) = waker.channel();
    debug!("overlay built");
    (
        Overlay {
            panel: build(mtm),
            message_receiver,
        },
        OverlaySink { message_sender },
    )
}
```

Each end lives on its own side: the receiver on the `Overlay`, which drains it, and the sender on the `OverlaySink`. There is no `Overlay::sink()` — a caller that wants another producer clones the sink it was handed, since `OverlaySink` is `Clone`.

### show / hide / pump / Drop

`show`/`hide` become sends, off any thread, with no libdispatch:

```rust
impl OverlaySink {
    /// Show the overlay with `text`, from any thread. The send wakes the main loop, so `pump` runs
    /// and the panel updates at once.
    pub fn show(&self, text: String) {
        let _ = self.message_sender.send(OverlayMsg::Show(text));
    }

    /// Hide the overlay, from any thread. A no-op if it is not up. The panel stays built.
    pub fn hide(&self) {
        let _ = self.message_sender.send(OverlayMsg::Hide);
    }
}
```

The panel mutation moves out of the dispatched block into a main-thread drain. The body is what `show`/`hide` did, minus the id lookup, since the panel is `self.panel`:

```rust
impl Overlay {
    /// Apply every queued show/hide to the panel. Call on the main thread, from `on_wake`.
    ///
    /// # Panics
    ///
    /// If called off the main thread, where the panel cannot be touched.
    pub fn pump(&self) {
        let mtm = MainThreadMarker::new().expect("Overlay::pump must run on the main thread");
        let Panel { panel, label } = &self.panel;
        for msg in self.message_receiver.try_iter() {
            match msg {
                OverlayMsg::Show(text) => {
                    label.setStringValue(&NSString::from_str(text.trim_end()));
                    label.sizeToFit();
                    resize_to_label(panel, label);
                    place(panel, mtm);
                    panel.orderFrontRegardless();
                    debug!(text, "overlay shown");
                }
                OverlayMsg::Hide => {
                    panel.orderOut(None);
                    debug!("overlay hidden");
                }
            }
        }
    }
}
```

`Drop` closes the panel it now owns, with no table to remove from:

```rust
impl Drop for Overlay {
    fn drop(&mut self) {
        self.panel.panel.close();
        debug!("overlay closed");
    }
}
```

## daemon.rs

`daemon.rs` builds the overlay with the same `waker` the title channel uses, keeps the `Overlay` for the panel's life, and hands the sink to `Boot`. It drains the overlay on each wake, beside the title.

Construction, before:

```rust
    let overlay = freddie_overlay::overlay();
    // ...
    let boot = Boot {
        // ...
        overlay: overlay.sink(),
    };
    // ...
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
    });
    // ...
    drop(overlay);
```

after:

```rust
    let (overlay, overlay_sink) = freddie_overlay::overlay(&waker);
    // ...
    let boot = Boot {
        // ...
        overlay: overlay_sink,
    };
    // ...
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
        overlay.pump();
    });
    // ...
    drop(overlay);
```

`OverlaySink` stops being `Copy` (a `Sender` is not `Copy`); it stays `Clone`. Today `perform_effect` takes the sink by value and the effect loop passes it every iteration, which only compiles because of `Copy`. After the change it takes a shared ref, matching `title_tx`:

`run_effect_loop` / `perform_effect`, before:

```rust
async fn run_effect_loop(
    // ...
    overlay: OverlaySink,
) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(
            effect,
            &emitter,
            &event_tx,
            &title_tx,
            windows.as_ref(),
            overlay,
        )
        .is_break()
        {
            break;
        }
    }
}

fn perform_effect(
    // ...
    title_tx: &freddie_main_loop::WakingSender<&'static str>,
    windows: Option<&WindowSink>,
    overlay: OverlaySink,
) -> ControlFlow<()> {
    // ...
    MercuryEffect::ShowOverlay(text) => overlay.show(text.to_owned()),
    MercuryEffect::HideOverlay => overlay.hide(),
    // ...
}
```

after:

```rust
async fn run_effect_loop(
    // ...
    overlay: OverlaySink,
) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(
            effect,
            &emitter,
            &event_tx,
            &title_tx,
            windows.as_ref(),
            &overlay,
        )
        .is_break()
        {
            break;
        }
    }
}

fn perform_effect(
    // ...
    title_tx: &freddie_main_loop::WakingSender<&'static str>,
    windows: Option<&WindowSink>,
    overlay: &OverlaySink,
) -> ControlFlow<()> {
    // ...
    MercuryEffect::ShowOverlay(text) => overlay.show(text.to_owned()),
    MercuryEffect::HideOverlay => overlay.hide(),
    // ...
}
```

## Delivery is prompt

`show`/`hide` send on a `WakingSender`, which wakes the main run loop after the send. So `nextEventMatchingMask` returns at once, `on_wake` runs `pump`, and the panel changes without waiting — what the GCD dispatch delivered, now with no `thread_local`. On a bare channel the overlay would lag until the next real event on the exact keystroke that summons it; the waking channel is why that does not happen.

## Docs that still describe the old path

### `docs/platform-apis.md`

The main-thread section still names two routes and says a channel drained in `on_wake` waits for a slice. After `wake-the-main-loop` and this change, the waking channel is the route. Replace the paragraph:

before:

```markdown
Work that must happen on main, from a thread that is not main, has two routes. `DispatchQueue::main().exec_async` runs a block promptly, because the main queue is drained from inside the run loop, but the block must be `'static` and `Send`, so it cannot carry a thread-bound value and has to find one already there. A channel drained in `freddie_main_loop`'s `on_wake` can carry anything, but waits for the current slice to end.
```

after:

```markdown
Work that must happen on main, from a thread that is not main, goes through a channel drained in `freddie_main_loop`'s `on_wake`. Build the channel with [`MainWaker::channel`](../crates/freddie_main_loop): the sender wakes the run loop on each send, so the value is applied at once rather than when the next real event arrives. The receiver stays on main with whatever non-`Send` handle it mutates (an `NSStatusItem` title, an `NSPanel`). That is how the menu-bar title and the overlay reach main from the worker.
```

(If the relative link form does not match how other `docs/` pages cite crates, name the types in prose the same way neighboring paragraphs do and skip the link.)

### `docs-website/docs/interacting-with-macos/the-menu-bar-and-the-overlay.md`

before:

```markdown
Building and moving the overlay panel is main-thread-only too, but the callers are not. `freddie_overlay::show` and `hide` are callable from any thread and marshal themselves with `DispatchQueue::main().exec_async`, so the effect loop calls them directly from the worker.
```

and the `main_loop.run` snippet plus the paragraph that still mentions a 100ms slice and the main queue.

after:

```markdown
Building and moving the overlay panel is main-thread-only too, but the callers are not. `OverlaySink::show` and `hide` are callable from any thread: they send on a waking channel, and `Overlay::pump` on main applies the change, so the effect loop calls the sink directly from the worker.
```

```rust
main_loop.run(|| {
    if let Some(name) = title_rx.try_iter().last() {
        menu_bar.set_title(Some(&format!(" {name}")));
    }
    overlay.pump();
});
```

```markdown
`run` pumps `NSApplication` events, `nextEventMatchingMask` then `sendEvent`, rather than a bare `CFRunLoop`. A bare `CFRunLoop` services run-loop sources, which covers the `NSWorkspace` notifications, but it never dispatches the window-server events that a status item's clicks and menu tracking need. The same pump runs `on_wake`, which drains the title and overlay channels. It sleeps until a real event or a posted wake, so an idle process costs nothing.
```

## What is deleted

- The `thread_local! { PANELS, NEXT_ID }` block.
- `OverlayId` and every use of it.
- `Overlay::sink`.
- The `dispatch2` dependency and import.
- The `std::cell::{Cell, RefCell}`, `std::collections::HashMap`, and `std::marker::PhantomData` imports.
- The `PhantomData` marker on `Overlay`; the owned `Panel` (`!Send`) keeps it on its thread.
- GCD / main-queue wording in the crate module docs, `docs/platform-apis.md`, and the website overlay page.
