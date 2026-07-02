# foreground events

Knowing which app is frontmost and getting an event when it changes, fed into the runner as an input alongside keys. A `freddie_foreground` crate will do it; not built yet (v1 fakes the report in the effect loop).

## What we need

- The current frontmost app at startup.
- An event each time the frontmost app changes, carrying enough to identify it (bundle id or process name).
- That identity mapped onto the app's `App` set (Chrome, Ghostty, Zed, Other for mercury) and sent into the event channel, where `Foregrounded` dispatch records it.

## Two ways to read it

- `active-win-pos-rs` (existing, cross-platform): returns the active window's app/process. No notifications, so poll on an interval (say 150ms), diff against the last app, and emit a foreground event on change. Plain Rust, works beyond macOS, at the cost of latency and idle polling.
- macOS `NSWorkspace` via `objc2`: `frontmostApplication` for the current app, and an observer on `NSWorkspace.didActivateApplicationNotification` for changes (the notification's `NSRunningApplication` gives `bundleIdentifier`, `localizedName`, `processIdentifier`). Event-based, no polling, macOS-only, lower-level (objc2 message sends), needs a run loop (the source thread has one).

No special permission either way.

## Mapping to App

The OS gives a bundle id (`com.google.Chrome`) or process name; the model wants `App`. A small table maps known bundle ids to `App::Chrome` and so on, everything else to `App::Other`. This table is app-specific (mercury's generic set versus figaro's bespoke set), so it belongs with the bindings, not in the crate. The crate should hand up a stable identifier (prefer bundle id over localized name, which is display text and can vary).

## In the runner

The foreground source is a second source thread (the poll loop, or the `NSWorkspace` observer's run loop) forwarding `ForegroundEvent`s into the same event channel as the keyboard. Dispatch treats them like any input (`Foregrounded => on_foregrounded` records the app). This is the decoupling in event-loop.md: nothing here reaches into state, it just produces events, and it is the counterpart to app-foregrounding.md, which triggers the change this observes.

## Open questions

- `active-win-pos-rs` (poll, cross-platform) vs `objc2` `NSWorkspace` (notify, macOS-only); the poll interval if polling.
- Where the bundle-id to `App` map lives (with the app's bindings), and how figaro overrides it.
- Does `active-win-pos-rs` expose a stable app identifier (bundle id) or only a title/process name?
- Whether "window changed within the same app" matters, or only "app changed."
- Debouncing rapid app switches so a flurry of foreground events does not thrash the layer.
