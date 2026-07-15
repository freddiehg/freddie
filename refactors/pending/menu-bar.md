# menu bar

An icon in the macOS menu bar for as long as mercury runs, with a dropdown that quits. Later the title can optionally mirror model state.

## Use `tray-icon`

`tray-icon` (Tauri's crate, with `muda` as the menu layer) wraps `NSStatusItem` and already gives us `set_title`, a menu, and click events. The icon-plus-Quit version is small:

- Build a `muda::Menu` with one `MenuItem`, "Quit".
- Set it on a `tray_icon::TrayIcon` with a title or a template image. Create it once and hold it for the whole run so the icon is present whenever the keyboard grab is active.
- On the menu-event channel, when Quit fires, send the event that a handler turns into `MercuryEffect::Kill`.

We build the menu once, so there's no objc2, no target/action, and no menu diffing.

## The run loop is the real work

Everything else here is a handful of lines; the run loop is where the effort goes.

`main.rs` already runs a `CFRunLoop` on the main thread (`freddie_main_loop`), with the tokio event and effect loops on a spawned thread and the keyboard tap on its own thread. That structure exists for the app-nav watcher, so a run loop already owns the main thread.

`tray-icon` is built around `winit` and expects the main thread to have an `NSApplication`-style event loop, because a status item's button and menu tracking dispatch `NSEvent`s that `NSApplication` pumps. A bare `CFRunLoop::run_in_mode` delivers `NSWorkspace` notifications, which is why the app-nav watcher works, but it may not pump status-item clicks. So the open question is whether the current `CFRunLoop` drives `tray-icon`'s events, or whether `freddie_main_loop` has to switch to `NSApplication::run` with an accessory activation policy (so there's no Dock icon). We should settle that before adding the dependency, since it decides whether this is a drop-in or a main-loop change.

## A click is another event source

`tray-icon` delivers menu events on a channel. We forward them into the same `event_tx` the keyboard and app-nav sources feed, so a click enqueues a `MercuryEvent` and the tokio loop dispatches it like any other event. Quit then becomes the event a handler maps to `MercuryEffect::Kill`, the same effect that `escape` then `q` produces. The point of a mouse-reachable Quit is recovery: it works even when the grabbed keyboard doesn't.

## Later: the title mirrors state

We don't need this for icon-plus-Quit; it's additive.

`Mercury` grows a pure `label(&self) -> String` derived from the state tree (the active layer, and for the in-app layer the foregrounded app), tested alongside the transition tests. The event loop calls `bar.set_title(m.label())` after each dispatch, marshalled to the main thread. If the dropdown items should also depend on state, a `menu(&self) -> Menu` returns them as inert data and the loop rebuilds the small menu each time.

This is the React model: `label` and `menu` are the render function, pure over state, returning a description of the view; the platform crate is the renderer that paints that description. State is the single source of truth, the view is a projection of it, and the model never touches AppKit. React reconciles a virtual DOM against the last one to compute a minimal patch; at this size there's nothing to diff, so the loop just repaints (`set_title`, rebuild the menu) on every dispatch. If the menu ever grows enough that rebuilding it flickers or costs too much, that's the point to add diffing, and the render-function shape is already what makes diffing possible.
