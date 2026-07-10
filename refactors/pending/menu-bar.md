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

`set` mutates the menu bar item directly. No file, no external renderer. The backend is a native `NSStatusItem`, below.

## The backend: a native NSStatusItem

We own the menu bar item ourselves rather than routing through SwiftBar. The item is an `NSStatusItem` from `NSStatusBar.systemStatusBar`; we hold it and mutate its button's title imperatively. `MenuBar::set` is a `setTitle:` on the button. Updates are push: the moment `label` changes, the title changes. No file, no poll, no separate process.

Raw path with `objc2` + `objc2-app-kit`, at the selector level (exact Rust signatures churn across versions and are not compile-checked here, so this is the logic, not a snippet to paste):

- `NSApplication::sharedApplication`, activation policy `Accessory` so there is no dock icon.
- `NSStatusBar::systemStatusBar()`, then `statusItemWithLength:` with `NSVariableStatusItemLength` (`-1.0`).
- Grab `.button()`, call `setTitle:` with an `NSString`. Call it again anytime to update.
- `app.run()`.

`tray-icon` (Tauri's crate) wraps `NSStatusItem` cross-platform and gives `tray.set_title(Some("..."))`, less boilerplate but more dependency than we want for rendering text. Default to raw objc2; reach for `tray-icon` only if the raw path fights us.

## The one real cost: the main-thread run loop

`NSStatusItem` is an AppKit UI object, so it must be created and mutated on the main thread, and keeping it live means the process runs an AppKit run loop (`NSApplication::run`) on the main thread. That is the whole cost of going native, and it is a one-time structural change, not per-update overhead.

mercury today runs `tokio` (`current_thread`) on the main thread and the keyboard tap on its own thread with its own `CFRunLoop`. Adding the status item inverts main-thread ownership: AppKit takes the main thread via `app.run()`, and the tokio event/effect loops move to a spawned thread. The keyboard tap is unaffected; it is already off-main on its own run loop.

The crate hides the main-thread affinity. `MenuBar::new` is created on the main thread (objc2's `MainThreadMarker` enforces this at the type level, which is the reason to prefer objc2 over hand-rolled FFI). `set`, called from the tokio thread after a dispatch, marshals the update to the main queue (`dispatch2`'s main-queue async, or a channel the run loop drains). So the event loop calls `bar.set(state.label())` from wherever it runs and the crate gets it onto the main thread.

## Rejected: SwiftBar

SwiftBar was the incumbent (a plugin re-reading a file on an interval). Rejected because we do not want a SwiftBar-launched subprocess resident and polling. The file read was never the cost; the poll interval was, and to catch keypress-driven changes a poll would have to run at roughly keypress latency (~30ms), which is absurd idle work for a status string. SwiftBar's streaming-plugin mode (long-lived process, print a block then a `~~~` line, no interval) would have fixed the latency without polling, but it still means a resident SwiftBar subprocess and a topology question (is mercury the plugin SwiftBar launches, or does a thin plugin stream over IPC). Native `NSStatusItem` is simple enough that neither is worth it: we own the item, push updates directly, and drop SwiftBar, the file, and the poll together.

## Open

- Diffing in the event loop: `set` only on a changed `label`. Trivial, but decide whether the crate dedupes or the caller does.
- `label`'s vocabulary: exact strings per layer and per app, and whether the in-app layer shows the app name or a fixed `APP`.
- Confirm the objc2 / objc2-app-kit / dispatch2 versions and the exact `setTitle:` and main-queue-dispatch call sites against the pinned crates before writing the FFI.
- Where `app.run()` and the tokio threads are wired: mercury's `main` currently owns the tokio runtime on the main thread, so moving tokio to a spawned thread and giving main to AppKit is a `main` restructure, tracked here so it is not a surprise.
