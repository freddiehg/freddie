---
title: The Crates
sidebar_position: 7
---

# The Crates

- `bind`, `bind_macro`, `derive_support`: bindings from a trigger to a handler, and the derives that build them.
- `laserbeam`: the typed mutable path the bindings are built over.
- freddie: the framework itself, over the two above.
- `freddie_keys`: keys, presses, and modifier flags.
- `freddie_keyboard`: grabbing the keyboard and emitting keys.
- `freddie_app_nav`: foregrounding an app, and watching which one is frontmost.
- `freddie_windows`: placing the focused window.
- `freddie_menu_bar`, `freddie_overlay`, `freddie_main_loop`: the menu-bar item, the keymap overlay, and the run loop they need.
- `freddie_event_socket`: the loopback listener external events arrive on.
- `freddie_single_instance`: the lock that keeps one mercury running.
- `mercury`: the application.

The split is between the crates that name a platform API and the ones that do not.

`bind`, `bind_macro`, `derive_support`, `laserbeam`, `freddie_keys` and freddie itself name none. They depend on `syn`, `quote`, `tokio`'s sync primitives, and each other, and a freddie program on another OS keeps every one of them unchanged. `freddie_event_socket` is `tokio` and `tokio-tungstenite`, so it travels too, and `freddie_single_instance` already picks its lock directory per OS, with macOS, Windows and other-unix arms.

`freddie_app_nav`, `freddie_main_loop`, `freddie_overlay` and `freddie_windows` are macOS and nothing else: `objc2` and AppKit, plus the Accessibility API for window placement. Each relaxes the workspace's `forbid(unsafe_code)` to `deny` for that reason, and allows the calls one at a time with a SAFETY comment at each site. `freddie_menu_bar` sits on `tray-icon`, which has backends beyond this one, but the contract it documents (create it on the main thread, after `NSApp` exists) is macOS's.

`freddie_keyboard` is the one whose shape says the platform is meant to move. It has a `sys` module holding a single backend behind `#[cfg(target_os = "macos")]`, `core-graphics` gated to that target in its manifest, and a `compile_error!` for anything else. `Emitter`, `Interceptor` and `intercept` are the three names a second backend would have to supply.

A non-macOS freddie program keeps the model, the derives, the paths and the keys, and rewrites the crates that talk to the window server.
