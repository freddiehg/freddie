# menu bar

An icon in the macOS menu bar for as long as mercury runs, with a dropdown that quits. Later, optionally, the title can mirror model state.

## Use `tray-icon`

`tray-icon` (Tauri's crate, with `muda` as the menu layer) wraps `NSStatusItem` and gives `set_title`, a menu, and click events already wired. The icon-plus-Quit version is small:

- Build a `muda::Menu` with one `MenuItem`, "Quit".
- Set it on a `tray_icon::TrayIcon` with a title or a template image, created and held for the whole run so the icon is present whenever the keyboard grab is active.
- On the menu-event channel, when Quit fires, send the event a handler turns into `MercuryEffect::Kill`.

No objc2, no target/action, no menu diffing. The menu is built once.

## The one real cost: the run loop

This is the actual work; the menu itself is a handful of lines.

`main.rs` already runs a `CFRunLoop` on the main thread (`freddie_main_loop`) with the tokio event and effect loops on a spawned thread and the keyboard tap on its own thread. That structure exists for the app-nav watcher, so main-thread ownership already belongs to a run loop.

`tray-icon` is built around `winit` and expects the main thread with an `NSApplication`-style event loop, because a status item's button and menu tracking dispatch `NSEvent`s that `NSApplication` pumps. A bare `CFRunLoop::run_in_mode` delivers `NSWorkspace` notifications (which is why the app-nav watcher works) but may not pump status-item clicks. So the integration question is whether the current `CFRunLoop` drives `tray-icon`'s events, or whether `freddie_main_loop` must switch to `NSApplication::run` with an accessory activation policy (so there is no Dock icon). Resolve that before adding the dep; it decides whether this is a drop-in or a main-loop change.

## A click is an event source

`tray-icon` delivers menu events on a channel. Forward them into the same `event_tx` the keyboard and app-nav sources feed, so a click enqueues a `MercuryEvent` and the tokio loop dispatches it like any other event. Quit is then the event a handler maps to `MercuryEffect::Kill`, the same effect `escape` then `q` produces. Its value beyond the icon: a mouse-reachable Quit is a recovery path that does not depend on the grabbed keyboard still working.

## Later: the title mirrors state

Not needed for icon-plus-Quit; additive when wanted.

`Mercury` grows a pure `label(&self) -> String` derived from the state tree (the active layer, and for the in-app layer the foregrounded app), testable beside the transition tests. The event loop calls `bar.set_title(m.label())` after each dispatch, marshalled to the main thread. If dropdown items should also depend on state, a `menu(&self) -> Menu` returns them as inert data and the loop rebuilds the tiny menu each time. The model produces strings and inert commands; the platform crate paints them. No diffing at this size.
