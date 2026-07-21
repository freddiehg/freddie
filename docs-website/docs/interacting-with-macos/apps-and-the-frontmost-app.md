---
title: Apps and the Frontmost App
sidebar_position: 4
---

# Apps and the Frontmost App

`freddie_app_nav` does two things: it foregrounds an app, and it watches which one is frontmost.

## The watcher

`watch` registers a block with `NSWorkspace`'s own notification center for `NSWorkspaceDidActivateApplicationNotification`, passing no queue, so the block runs on the thread that posts the notification, which is main. The block reads the `NSRunningApplication` out of the notification's user info under `NSWorkspaceApplicationKey` and takes its `bundleIdentifier`.

That is one call per real activation. The notification is posted when the front app actually changes, so there is nothing to diff and no interval to tune.

The crate hands up a string and stops there. `mercury` supplies the callback that turns it into an event:

```rust
let _watcher = freddie_app_nav::watch({
    let event_tx = event_tx.clone();
    move |bundle_id| {
        let app = App::from_bundle_id(bundle_id);
        let _ = event_tx.send(foreground(app));
    }
});
```

`foreground(app)` is `MercuryEvent::Foreground(ForegroundEvent { app })`, and `App::from_bundle_id` is where a string becomes one of the apps `mercury` knows, or `App::Other`. The block is on the main thread, where callbacks are serialized, so it does one send and returns; the dispatch happens back on the worker thread.

Two things the caller has to do. `watch` reports changes and not the app that is already up, so the daemon seeds the state from `freddie_app_nav::frontmost()` before the loops start. And the returned `Watcher` has to be held for as long as the events are wanted: dropping it calls `removeObserver`, which is the only thing that stops the observation.

## One copy, at the root

The root holds the front app, and nothing else does:

```rust
pub struct Foreground {
    app: ForegroundedApp,
    navigating: bool,
}
```

The root binds the event to the one handler that writes it:

```rust
#[bind(
    Foregrounded => record_front_app,
    // ...
)]
```

`record_front_app` calls `set_front_app`, which records the app and clears `navigating`. Nothing else moves: foregrounding an app does not change which layer you are in.

The in-app layer stores no app at all. It declares a [virtual field](../architecture/virtual-fields.md), `#[derived_child(app_data)]`, and `app_data` reads the root's copy on every dispatch:

```rust
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    // AppLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    match root.foreground.confirmed() {
        Some(App::Chrome) => {
            Some(AppData::Chrome(ChromeApp::new()))
        }
        Some(App::Ghostty) => {
            Some(AppData::Ghostty(GhosttyApp::new()))
        }
        _ => None,
    }
}
```

So `r` refreshes only while Chrome is frontmost, and there is no copy in the layer to keep in sync.

`confirmed` returns `None` while a navigation is in flight. A nav choice foregrounds an app and marks `navigating`, and until the watcher reports back, `app` is still the previous one. Binding it in that gap would aim the in-app keys at the app being left. `Finder`, `Zed`, and `Other` bind nothing, so they get no level either.

## Foregrounding

Bundle identifiers are the identity in both directions. Display names are not: System Events calls Ghostty `ghostty` while the app calls itself `Ghostty`. `App::bundle_id` is the reverse of `App::from_bundle_id`, and the two round-trip. `App::Other` is not a specific app, so it has none.

```rust
pub fn foreground(bundle_id: &str) -> Result<(), NavError>;
```

It runs `open -b <bundle_id>`, which launches the app if it is not running and brings it to the front, so an app that is not running is not a special case. It is fire-and-forget and does not confirm that anything came up. `NavError::Spawn` is `open` failing to spawn; `NavError::Failed` is `open` exiting non-zero, which is what an unknown bundle id and a refused activation both look like from here.

The effect loop performs it on a detached thread, so `open` never delays a key the loop is about to emit, and an `App::Other` logs a warning and does nothing. Nothing waits on the result, because the watcher reports the app that actually came up and that is what the model records.
