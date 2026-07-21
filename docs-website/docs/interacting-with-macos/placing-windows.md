---
title: Placing Windows
sidebar_position: 5
---

# Placing Windows

In the resize layer, `up` maximizes a window, `right` resizes to the right half, and `left` resizes to the left half.

Each of those is a `MercuryEffect::Place(placement)`, and each also goes home, since placing a window is one decision rather than something you repeat. The effect loop maps `mercury`'s own `Placement` onto `freddie_windows::Placement` and performs it on a detached thread: a placement takes single-digit to low tens of milliseconds, which is long enough to delay a key the loop is about to emit.

## Finding the window

The Accessibility API is the only way to set a window's frame. `CGWindow` can read geometry and not write it.

`freddie_windows` walks down to the window in three steps:

```rust
let pid = NSWorkspace::sharedWorkspace()
    .frontmostApplication()?
    .processIdentifier();
let app = AXUIElementCreateApplication(pid);
// then kAXFocusedWindowAttribute off `app`
```

`AXUIElementCopyAttributeValue` with `kAXFocusedWindowAttribute` gives the focused window as a `+1` reference, which `place` releases when it is done. Nothing frontmost, or a frontmost app with no focused window, is `WindowError::NoFocusedWindow`.

What it then sets is two attributes, `kAXPositionAttribute` from a `CGPoint` and `kAXSizeAttribute` from a `CGSize`, each wrapped with `AXValueCreate`. Reading the position is the same call in the other direction, and that is how a window's monitor is decided.

## Coordinates and displays

`init` runs once, on the main thread, before any placement. `NSScreen` is `AppKit`, so reading the screens is main-thread-bound, and `place` runs off main. `init` reads every monitor, caches it, and registers an observer for `NSApplicationDidChangeScreenParametersNotification` that re-reads the cache whenever a monitor is plugged, unplugged, or rearranged. A display connected after launch is therefore a display `place` knows about.

The two coordinate systems disagree, and reconciling them is most of the math. `NSScreen` has a global origin at the bottom left with y increasing up; Accessibility has one at the top left with y increasing down. The flip is around the primary display's full height, not each screen's own:

```rust
Frame {
    x: rect.origin.x,
    y: primary_height - (rect.origin.y + rect.size.height),
    width: rect.size.width,
    height: rect.size.height,
}
```

Flipping around each screen's own height would put a monitor above or beside the primary at the wrong global y.

Each monitor is cached as two frames. `full` is the whole display, and it is what a window's top-left corner is tested against to decide which monitor the window is on; a corner on no monitor falls back to the first. `visible` is the area a placement fills, and it is `visibleFrame`, which is already the full frame minus the menu bar and the dock. So the menu bar never appears in `freddie_windows`' own arithmetic. `Maximize` is the visible frame exactly, and the halves split its width and keep its height and origin, which is why a placement on a second display fills that display rather than the one the app started on.

## Apps that refuse

The frame is set twice, in a loop:

```rust
for _ in 0..2 {
    // set kAXPositionAttribute, then kAXSizeAttribute
}
```

Some apps clamp a move against their current size, so the first position lands short of where it was asked to go and the second lands true.

Beyond that, an app is free to ignore what it is told. `AXUIElementSetAttributeValue` returns a status that `set_frame` does not read, so a window with a minimum size, a fixed size, or a full-screen space of its own ends up wherever the app decided, and `place` still returns `Ok`. The log line says the placement was performed, and it means the attributes were set, not that the window obeyed.

The errors that do surface are about not getting as far as the window: `NotTrusted` and `NoScreen` from `init`, `NotInitialized` when no monitor was ever cached, and `NoFocusedWindow`. The effect loop logs each of those as `place failed`.
