---
title: Interacting with macOS
sidebar_position: 1
---

# Interacting with macOS

`mercury` is macOS-only, and the crates it leans on are where every platform call lives. Keeping them at the edges is what lets the model stay pure and portable.

## Permissions

Accessibility is the permission `mercury` needs. Grant it in System Settings, under Privacy & Security, Accessibility, to the binary that runs: the grant follows the installed path, so `~/.cargo/bin/mercury` and a `target/debug/mercury` are two different grants.

Input Monitoring is not involved. `freddie_keyboard` creates its tap with `CGEventTapLocation::Session` and `CGEventTapOptions::Default`, which is an active tap that can consume and modify events, and that is what Accessibility gates. Input Monitoring gates listen-only taps, which only observe, so a process that remaps anything cannot be running on one.

Without the grant, `mercury start` still reports a pid. Taking the single-instance lock is the readiness signal and the daemon takes it first thing, before it measures the screens, shows an icon, or grabs the keyboard. The failures land in the log a moment later:

- `freddie_windows::init` returns `NotTrusted`, so the daemon logs `window placement unavailable` and carries on. Nothing is cached, and a later `Place` effect has no monitor to place within.
- `freddie_keyboard::intercept` fails, the daemon logs `could not intercept the keyboard`, and the worker returns. That drops the `Stopper`, which stops the main run loop, so the process exits rather than sitting there with a dead keyboard.

The failure is silent from the outside, so reset the grant with `tccutil reset Accessibility` when testing that path.

One more permission is asked for later and only in one place. Copying the front Chrome tab's URL falls back to `osascript` when nothing has reported one, and that subprocess needs Apple Events. Everything else about Chrome comes from the extension.

## In this section

- [Grabbing the Keyboard](./grabbing-the-keyboard.md)
- [Emitting Keys](./emitting-keys.md)
- [Apps and the Frontmost App](./apps-and-the-frontmost-app.md)
- [Placing Windows](./placing-windows.md)
- [The Menu Bar and the Overlay](./the-menu-bar-and-the-overlay.md)
- [The Chrome Extension](./the-chrome-extension.md)
- [Logging](./logging.md)
