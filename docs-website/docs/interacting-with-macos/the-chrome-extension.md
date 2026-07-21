---
title: The Chrome Extension
sidebar_position: 7
---

# The Chrome Extension

The extension at `./chrome-extension` reports the URL of the active tab. It sends one message and takes none, which is what per-site bindings are built on: the active tab is Chrome's to know, and no app-activation event carries it.

## Loading it

Chrome loads what `tsc` writes into `dist/`, so it has to be built before it will load at all:

```bash
cd chrome-extension
pnpm install
pnpm build
```

Then, at `chrome://extensions`, turn on Developer mode and use Load unpacked on that directory. Rebuilding does not reload the extension; press the reload arrow on its card after every build. `pnpm watch` rebuilds on save.

The manifest asks for `tabs`, `storage`, and host permission on `http://127.0.0.1/*`, and nothing else.

## Getting the URL across

`freddie_event_socket` binds a loopback WebSocket and hands each text frame to a callback. It knows nothing about any particular event type, so the caller decides what a frame means.

```rust
let _socket = freddie_event_socket::listen(port, move |frame| {
    // parse, then send on the event channel
})?;
```

The extension pushes on every tab switch and every navigation, so `mercury` never asks and never polls. What arrives becomes an ordinary event:

```rust
pub struct TabEvent {
    pub url: String,
}
```

Two things guard the socket. It binds loopback only, and the handshake refuses browser origins: a WebSocket handshake is exempt from the same-origin policy, so without that check any page in any open tab could drive it. Frames past 64 KB close the connection that sent them, so a client cannot make the process allocate without bound.

The default port is 3883. If `mercury` was started with `--port` or `MERCURY_PORT`, set the matching port in the extension's options.

The wire types in `src/wire/` are generated from `crates/mercury/src/external.rs` and checked in. Pre-commit regenerates them on any Rust change and refuses the commit if the result differs, so a stale copy cannot ship.

## Per-site bindings

The site layer holds what the site in the front tab can do. It is deliberately separate from the in-app layer: in-app is what Chrome the application can do and holds whatever is true of every tab, while this changes as you move between tabs without the frontmost app changing at all.

It stores no site. `site_data` is a [virtual field](../architecture/virtual-fields.md) that reads the front tab's URL off the root on every dispatch, so switching tabs while sitting in the layer changes what is bound with no event of its own.

A site with no bindings resolves to no level, and the layer's own bindings are all that is left: `esc` home, `t` to typing, `o` for the overlay. The overlay follows the same way, with `overlay_for` picking the keymap for the site in front.

## Checking it works

With `mercury` running, follow the log and switch tabs. Each switch should add a record whose state carries the new URL:

```
Foreground { app: Chrome(ForegroundedChrome { url: Some("https://claude.ai/new") }), ... }
```

Nothing at all means the frame never arrived. In order of likelihood: `mercury` is not running, the port does not match, or the service worker threw. Its console is behind the "service worker" link on the extension's card, and its errors appear there rather than in any page's console.
