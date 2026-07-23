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

## The shape

Each `Overlay` owns its `Panel` and the receiving end of a channel. `OverlaySink` holds the sending end, which is `Send` and `Clone`. `show`/`hide` send a message; the main thread drains it and mutates the panel it owns. There is no shared table, so there is no id and no lookup.

`crates/freddie_overlay/src/lib.rs`. The `thread_local!` block and `OverlayId` are deleted, along with the `dispatch2` and `Cell`/`RefCell` imports. A message type replaces them:

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
    /// Drained by [`Overlay::pump`] on the main thread. The sinks hold the senders.
    incoming: Receiver<OverlayMsg>,
}
```

`OverlaySink`, before:

```rust
#[derive(Clone, Copy, Debug)]
pub struct OverlaySink {
    id: OverlayId,
}
```

after:

```rust
/// The handle showing and hiding go through. `Send` and `Clone`, so any thread can hold one; the
/// panel it drives is on the main thread, reached by sending rather than by touching it.
///
/// Safe to keep past its [`Overlay`]: once the overlay is dropped the receiver is gone, and a send
/// is a harmless error, which is what hiding an already-gone overlay would have been.
#[derive(Clone, Debug)]
pub struct OverlaySink {
    outgoing: Sender<OverlayMsg>,
}
```

`overlay()` builds the panel and the channel, before:

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
```

after:

```rust
pub fn overlay() -> Overlay {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let (_outgoing, incoming) = std::sync::mpsc::channel();
    debug!("overlay built");
    Overlay {
        panel: build(mtm),
        incoming,
    }
}
```

The sink carries a sender, so `sink()` clones one out. `overlay()` keeps one sender alive on the `Overlay`? No: the `Overlay` holds only the receiver, and `sink()` is the sole source of senders. Since `sink()` needs a sender to clone, the `Overlay` holds the original:

```rust
pub struct Overlay {
    panel: Panel,
    outgoing: Sender<OverlayMsg>,
    incoming: Receiver<OverlayMsg>,
}

impl Overlay {
    #[must_use]
    pub fn sink(&self) -> OverlaySink {
        OverlaySink { outgoing: self.outgoing.clone() }
    }
}
```

`show`/`hide` become sends, off any thread, with no libdispatch:

```rust
impl OverlaySink {
    /// Show the overlay with `text`, from any thread. Applied on the main thread's next wake.
    pub fn show(&self, text: String) {
        let _ = self.outgoing.send(OverlayMsg::Show(text));
    }

    /// Hide the overlay, from any thread. A no-op if it is not up. The panel stays built.
    pub fn hide(&self) {
        let _ = self.outgoing.send(OverlayMsg::Hide);
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
        for msg in self.incoming.try_iter() {
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

`daemon.rs` drains the overlay on each wake, beside the title:

```rust
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
        overlay.pump();
    });
```

## The latency this trades for

The dispatched block ran on the main queue's next turn; `pump` runs on the main loop's next wake, which is at least every `SLICE` (100ms) and sooner on any `AppKit` event. So a show can lag up to `SLICE`. This is the same latency the menu-bar title already takes through `on_wake`, and it is acceptable there. The overlay is a glance-hint on a toggle or a dwell, not a per-frame element, so it takes the same deal.

If that lag ever shows, the fix is to wake the run loop on send the way `Stopper` does, not to go back to the table: the sink would hold a `CFRunLoop` handle and call `wake_up` after the send. That is more machinery than `on_wake`, so it is held until the latency is a real complaint rather than a hypothetical.

## What is deleted

- The `thread_local! { PANELS, NEXT_ID }` block.
- `OverlayId` and every use of it.
- The `dispatch2::DispatchQueue` dependency and import, and the `std::cell::{Cell, RefCell}` import.
- The `PhantomData` marker on `Overlay`; the owned `Panel` (`!Send`) keeps it on its thread.

`OverlaySink` stops being `Copy` (a `Sender` is not `Copy`); it stays `Clone`. Its one holder, `daemon.rs`'s `Boot { overlay: overlay.sink() }`, calls `sink()` once, so nothing depends on the `Copy`.
