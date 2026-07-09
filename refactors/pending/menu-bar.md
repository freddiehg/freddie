# menu bar status

Show the current state in the macOS menu bar: which layer Mercury is in, which app is foregrounded, whatever we want. The menu bar is a status line for the model, so glancing up answers "what will my next key do".

## The label comes from state

`Mercury` grows one method:

```rust
impl Mercury {
    pub fn label(&self) -> String { ... }
}
```

It returns a `String` derived purely from the state tree: the active layer, and for the in-app layer the foregrounded app. Something like `HOME`, `NAV`, `TYPING`, `chrome`. Pure function of `&self`, no I/O, no side effects, trivially testable next to the transition tests. It is the model's half of the contract, exactly parallel to `App::from_name`: the model decides what string represents its state, the renderer decides how to paint it.

The rendering is not the model's job and does not belong in `mercury`. `label` produces text; a separate crate puts text in the menu bar. That split is the whole design: the model stays a pure state machine, the platform code stays quarantined behind a crate boundary.

## The renderer is its own crate

The "put this text in the menu bar" function lives in its own crate, e.g. `freddie_menu_bar`, sibling to `freddie_app_nav` and `freddie_keyboard`. Public surface is roughly:

```rust
let bar = freddie_menu_bar::MenuBar::new()?;
bar.set("HOME");   // call again anytime to update
```

It takes an arbitrary string. It knows nothing about `Mercury`, `Layer`, or `label`. That keeps the platform FFI in one crate that the model never depends on, and lets the same renderer show arbitrary things later (a timer, a count, an error) without touching `mercury`. The event loop is the glue: after each dispatch it computes `state.label()` and, when it changed, calls `bar.set(...)`. Recomputing every dispatch is free (`label` is microseconds); the only reason to diff is to avoid redundant renders.

Where `set` goes is a backend decision the crate hides. Two families, below. The recommendation is the streaming SwiftBar plugin.

## Reframe: the file read is not the cost

The current itch is "re-reading a file to render is inefficient". That is mostly a misdiagnosis. A stat plus a read of a tiny file is microseconds. The latency you feel is SwiftBar's poll interval: it only re-runs the plugin every N seconds, and your update waits for the next tick. The read is not what is slow; the interval is. If latency is the only complaint, it is fixable without leaving SwiftBar and without removing the file.

## Keeping SwiftBar as the renderer

SwiftBar stays the thing that draws the menu bar item; we change how it learns about updates.

- Push-refresh via URL scheme. `open "swiftbar://refreshplugin?name=<plugin>"` forces an immediate refresh instead of waiting for the interval. It still re-runs the plugin, which re-reads the file, so it does not remove the file dependency, but the delay is gone. Cheap, keeps the current plugin as is.
- Streaming plugin. SwiftBar can run a plugin as a long-lived process and read its stdout continuously. The plugin prints a block, then a line containing only `~~~`, and SwiftBar re-renders. No file, no interval, fully push. Our Rust binary becomes the plugin: it stays alive and prints when it wants to update. This is the actual answer to "eliminate the file and the poll" while keeping SwiftBar's rendering. Verify the exact metadata declaration for marking a plugin streamable against current SwiftBar docs before building: the `~~~` separator is confirmed, the precise header token that flags a plugin as streamable is not, and it is not worth guessing.

Streaming is strictly better than URL-refresh for the stated goal (no file, no interval) and is a small change. If we keep SwiftBar, this is where to stop.

Topology caveat, for the "## Open" list: in streaming mode SwiftBar launches and owns the plugin process, reading its stdout. That is a different process shape than "mercury is the daemon and owns the keyboard tap". Either mercury itself is the plugin SwiftBar launches (it then also owns stdout as the render channel), or a thin plugin process streams on mercury's behalf over an IPC channel. Which one drives whether `freddie_menu_bar`'s backend is "write framed blocks to stdout" or "talk to the mercury process". Decide this before committing to streaming.

## Owning the menu bar item ourselves

If we want control SwiftBar does not expose (click handlers, custom drawing, submenus built imperatively), we drop SwiftBar and own the item.

The item is an `NSStatusItem` from `NSStatusBar.systemStatusBar`. We hold it and mutate its button's title imperatively. The cost is real: we now own an `NSApplication` run loop on the main thread, i.e. a long-lived agent process, for what is fundamentally "set a string in the menu bar".

Raw path with `objc2` + `objc2-app-kit`, at the selector level (exact Rust signatures churn across versions and are not compile-checked here, so this is the logic, not a snippet to paste):

- `NSApplication::sharedApplication`, activation policy `Accessory` so there is no dock icon.
- `NSStatusBar::systemStatusBar()`, then `statusItemWithLength:` with `NSVariableStatusItemLength` (`-1.0`).
- Grab `.button()`, call `setTitle:` with an `NSString`. Call it again anytime to update.
- `app.run()`.

AppKit mutations must happen on the main thread. If update logic lives on a worker thread, funnel updates to the main queue (`dispatch2`'s main-queue async, or a channel drained by a run-loop source). objc2's `MainThreadMarker` enforces this at the type level in recent versions, which is the reason to prefer objc2 over hand-rolled FFI here.

Wrapper alternative: `tray-icon` (Tauri's crate) wraps `NSStatusItem` cross-platform and gives `tray.set_title(Some("..."))`. Less boilerplate, but it still needs an event loop and pulls in more than we want if all we render is text.

## Recommendation

Streaming SwiftBar plugin. We keep the renderer, get push updates, write the daemon in Rust, and do not take on a Cocoa run loop for setting a string. Go native only if we want control SwiftBar does not expose. Whichever backend wins, it hides behind `freddie_menu_bar::MenuBar::set`, and `Mercury::label` stays the same pure function feeding it.

## Open

- Streaming topology: is mercury the plugin SwiftBar launches, or does a thin plugin stream on mercury's behalf over IPC (see the caveat above). Drives the crate's backend shape.
- Confirm the SwiftBar streamable-plugin header token in current docs; `~~~` is confirmed, the header is not.
- Diffing in the event loop: `set` only on a changed `label`. Trivial, but decide whether the crate dedupes or the caller does.
- `label`'s vocabulary: exact strings per layer and per app, and whether the in-app layer shows the app name or a fixed `APP`.
