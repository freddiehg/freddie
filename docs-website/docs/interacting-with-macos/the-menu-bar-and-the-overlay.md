---
title: The Menu Bar and the Overlay
sidebar_position: 6
---

# The Menu Bar and the Overlay

`mercury` creates a menu bar item, which shows the current layer name and exposes a quit option. If you end up with a non-responsive keyboard while iterating, that is how you save yourself.

From any layer except typing, `o` shows an overlay of what is bound.

## The main thread

`AppKit` delivers its callbacks on the main thread's run loop, and a run loop delivers only while some thread is inside it. Three things here depend on that.

Creating the status item is main-thread-only, and so is `set_title` on it: an `NSStatusItem` may only be touched from the thread that built it, and `MenuBar` is `!Send` so that the type says as much. `NSApp` has to exist first, which is `freddie_main_loop::init_menu_bar_app`, called once on main before the item is built. It sets the accessory activation policy, keeping the process out of the Dock and the cmd-tab switcher, and calls `finishLaunching`, which posts the launch notifications `AppKit` expects before it will deliver events.

Building and moving the overlay panel is main-thread-only too, but the callers are not. `freddie_overlay::show` and `hide` are callable from any thread and marshal themselves with `DispatchQueue::main().exec_async`, so the effect loop calls them directly from the worker.

The main thread gets there by parking in `MainLoop::run` and doing nothing else:

```rust
main_loop.run(|| {
    if let Some(name) = title_rx.try_iter().last() {
        menu_bar.set_title(Some(&format!(" {name}")));
    }
});
```

`run` pumps `NSApplication` events, `nextEventMatchingMask` then `sendEvent`, rather than a bare `CFRunLoop`. A bare `CFRunLoop` services run-loop sources, which covers the `NSWorkspace` notifications, but it never dispatches the window-server events that a status item's clicks and menu tracking need. The same pump is what services the main queue the overlay dispatches onto. It wakes at least every 100ms to check whether it has been stopped, and again whenever an event arrives.

That leaves the main thread a doorman. A callback on it is serialized against every other one, so each does one send and returns, and the real work happens on the worker.

## The layer name

The name is an effect, not a read. The layer changes in exactly one place, `Mercury::set_layer`, which already returns the effects a transition implies, so the name rides out with them:

```rust
effects.push(MercuryEffect::ShowLayer(self.layer.name()));
```

Nothing else produces `ShowLayer`, so the item and the model cannot disagree about which layer is active. `Layer::name` is a `match` over the layer enum, which is what forces a new layer to have a name rather than defaulting to something.

The effect loop, on the worker, cannot touch the status item, so it sends the name over a `std::sync::mpsc` channel. The receiving end is the main thread, which is not in the tokio runtime, which is why this one channel is not tokio's. The main loop drains it on its next wake with `try_iter().last()`, so only the last name in a batch is drawn: intermediate layers in one dispatch are not worth showing. The cost is up to 100ms of latency on a layer change, which is invisible on a passive indicator.

The daemon sends one name by hand before the loops start, because nothing has transitioned yet and no `ShowLayer` has been produced:

```rust
let _ = title_tx.send(mercury.layer().name());
```

Quit is the other half of the item. Its handler runs on the main thread and sends the same `Quit` event any source sends, which the model turns into `Kill`. That ends the effect loop, which releases the keyboard and stops the main loop, so the mouse-reachable way out runs the same destructors the keyboard one does.

## What the overlay shows

The overlay is an external `AppKit` window and not model state, so it is driven entirely by effects: `ShowOverlay(text)` puts it up and `HideOverlay` takes it down. Its only trace in the tree is one field on the root, `overlay: Option<TimerGuard>`, the guard for the pending hide of whatever is up.

The text is a keymap file per layer, checked in beside the layer it describes and pulled in with `include_str!`. It is not derived from the bind tree. A table written as string literals cannot be read as the table it is, and lining up the columns is the point, so what is in the file is what the panel draws:

```
  RESIZE
  ────────────────────
  ↑    maximize
  ←    left half
  →    right half
  t    typing
  esc  home
```

Choosing the file is `Layer::overlay_content`, which takes the active layer and the root's foreground. Most layers are one file. The in-app layer is not, because its bindings are the app's: `i` in Ghostty and `i` in Chrome are different keymaps, so it picks by the confirmed front app and falls back to a file listing the in-app layer's own keys for an app that binds nothing. The site layer picks by the front tab's host, the same way.

`o` is bound to `toggle_overlay` in each layer that binds keys, not once at the root: a root binding would fire in typing too, and you could never type the letter. `toggle_overlay` reads the content once, pushes `ShowOverlay` and a 10-second hide timer, and stores that timer's guard. Pressing `o` again takes it down, so the key you press to ask what is bound is the key you press when you are done reading. `set_layer` hides it too, so leaving a layer takes its keymap with it.

Content is read once, when `o` shows it, so an overlay that is up can go stale: a front-app change does not redraw it. The dwell takes it down, or the next `o` shows the current one. It is a hint you asked for rather than a live view.

Drawing it is `freddie_overlay`: a borderless, non-activating `NSPanel` above the menu bar level, translucent dark, monospaced white text, sized to the text so more rows make a taller panel, placed against the right edge of the main screen. It ignores mouse events and does not hide when the app under it deactivates, so it reads as part of the screen rather than as a window.
