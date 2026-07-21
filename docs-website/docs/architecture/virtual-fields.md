---
title: Virtual Fields
sidebar_position: 6
---

# Virtual Fields

Nesting enums runs into a limitation. How do you handle the currently foregrounded app, which is only relevant in the inapp layer?

`struct AppLayer` could have `#[resolve_into] currently_foregrounded_app: CurrentlyForegroundedApp`, and that would work. But it means that when you navigate to the inapp layer, you must already know the foregrounded app. Discovering it at that moment is impure, and thus violates one of the basic tenets of freddie: `state.handle` is pure. Copying it into the layer on transition works, but then the state exists in two places and both have to be kept current.

The solution freddie offers is virtual fields.

A virtual field is a child level computed during dispatch instead of stored in the state. `AppLayer` declares one with `#[derived_child(app_data)]`. `app_data` is a function that returns a struct implementing `Bind`:

```rust
/// Reads the foregrounded app, the only copy, and builds the level for it.
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    // AppLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    match &root.foreground {
        Foregrounded::Chrome => Some(AppData::Chrome(ChromeApp::new())),
        Foregrounded::Ghostty => Some(AppData::Ghostty(GhosttyApp::new())),
        _ => None
    }
}
```

When dispatch reaches `AppLayer`, it calls `app_data`, which walks up to the root, reads the one copy of the frontmost app, and hands back the level to descend into. So `r` is bound to refresh only while Chrome is frontmost. On the next event `app_data` is called again, so bindings are never stale.

## Persistence

TODO: what happens to state held on a derived level between events.
