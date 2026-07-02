# foregrounding apps

Performing the `Foreground` effect: bringing an app to the front, launching it if needed. The counterpart to foreground-events.md, which reports the result.

## What the effect does

`Foreground(App)` asks the OS to make an app frontmost. It does not change state: the foreground watcher (foreground-events.md) reports the app actually coming up as a normal event, and that is what records it. So this is fire-and-forget, and it can fail (app missing, activation refused) without corrupting state.

## Ways to do it (Rust)

- `open -a <AppName>` via `std::process::Command`: launches the app if not running and brings it to the front if it is. Plain Rust, no FFI, macOS `open`. Simplest, and it both launches and activates. Downsides: by app name rather than bundle id, spawns a process, and gives little control or feedback.
- `NSRunningApplication` via `objc2`: find the running instance by bundle id and call `activate(options:)`. Precise, no subprocess, but only activates an already-running app (does not launch) and is objc2 FFI.
- `NSWorkspace.openApplication(at:configuration:)` via `objc2`: launches by URL or bundle id with a completion handler. Launches and activates, event-based, macOS-only, objc2.

`open -a` covers both launch and activate with no FFI, which fits "prefer existing and Rust"; `objc2` is the trade for precision and no subprocess.

## Sonoma activation

Recent macOS tightened cross-app activation: an app cannot always force another to the front without user involvement. `open -a` and `NSWorkspace.openApplication` generally still bring the app up; a bare `NSRunningApplication.activate` may be limited. Either way the watcher confirms the real frontmost app, so the runner never assumes the activation succeeded.

## App identity

Foregrounding needs the app's launch identity: a name for `open -a`, a bundle id for objc2. Chrome is `com.google.Chrome`, and so on. Same table concern as foreground-events.md, and app-specific (mercury generic, figaro bespoke).

## In the runner

The effect loop performs `Foreground(app)` by whichever mechanism and returns immediately. It synthesizes no follow-up event; the foreground source produces one when (and if) the app actually comes up. v1 fakes that by re-injecting a foreground event in the effect loop, which this replaces with the real activation plus the real watcher.

## Open questions

- `open -a` (no FFI, launches and activates, by name) vs `objc2` (`NSRunningApplication` / `NSWorkspace`, precise, bundle id).
- Handling activation the OS refuses (Sonoma): rely on the watcher, or surface a failure to the model?
- Launch-if-not-running: needed, or assume the app is already running?
- The name and bundle-id table, shared with foreground-events.md.
- Do we ever foreground something that is not a whole app (a window, a Space), or is app-level enough?
