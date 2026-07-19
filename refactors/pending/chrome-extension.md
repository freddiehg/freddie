# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and only the browser knows it. This extension is the bridge, over the loopback WebSocket mercury listens on.

It is Rust compiled to wasm, so the wire types are mercury's own types rather than a copy of them. Nothing is generated, nothing is hand-translated, and a frame the extension sends is built by the same `serde` impl mercury parses it with. A field renamed on one side stops compiling on the other.

This doc owns the browser side of one direction: pushing the frontmost tab's URL up. `external-events.md` owns mercury's side. Commands going the other way, and the token they need, are `external-effects.md`; nothing here anticipates them.

## What wasm costs

A `wasm32-unknown-unknown` toolchain, `wasm-bindgen-cli`, a build step before the extension can be loaded, a few hundred kilobytes of binary for what would be forty lines of JavaScript, and a CSP relaxation (`wasm-unsafe-eval`, below). What it buys is that `IncomingEvent` has exactly one definition in the repository. As the vocabulary grows past one variant with one string field, and especially once `external-effects.md` adds commands and replies in both directions, that stops being a rounding error.

## The layout

`chrome-extension/` at the top level, beside `crates/`, and it is its own cargo workspace.

That last part is a build requirement, not a preference. A `cdylib` depending on `web-sys` only compiles for `wasm32-unknown-unknown`, and the root workspace's `cargo clippy --all-targets` and `cargo test --all` build every member for the host, which is macOS. Listing it as a member would break both, and the pre-commit hooks with them.

```
chrome-extension/
  README.md            how to build, install, and check that it works
  Cargo.toml           its own [workspace], not a member of the root one
  src/lib.rs           the socket, the reconnect, and the frame encoding
  js/background.js     registers the chrome listeners and calls into wasm
  js/options.js        the port setting
  manifest.json
  options.html
  pkg/                 wasm-bindgen output, gitignored
```

The root `Cargo.toml` gains nothing. `chrome-extension/Cargo.toml` opens with an empty `[workspace]` table, which is what tells cargo it is a workspace root rather than a member of the one above it.

## Change 1: the wire types move to their own crate

A prefactor, and the thing that makes the rest possible. `IncomingEvent` and its payloads live in `crates/mercury/src/external.rs` today, and mercury does not compile for wasm.

New `crates/freddie_wire`, whose only dependency is `serde`:

```toml
[package]
name = "freddie_wire"
description = "The vocabulary mercury and its clients speak over the event socket."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }

[lints]
workspace = true
```

`crates/freddie_wire/src/lib.rs` takes what `external.rs` has now, verbatim:

```rust
//! The vocabulary mercury and its clients speak over the event socket.
//!
//! Its own crate because both ends depend on it and they do not share a target: mercury is a macOS
//! binary, and the Chrome extension is `wasm32-unknown-unknown`. Nothing here may pull in anything
//! that will not build for both, which in practice means `serde` and nothing else.

/// Everything an outside process may say to mercury. A sender cannot say anything else, so remote
/// key injection and remote quit are unrepresentable rather than filtered.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {
    /// The front browser tab's URL changed.
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct TabMessage {
    pub url: String,
}
```

`Serialize` is new. mercury only ever deserializes these, but the extension only ever serializes them, and one derive pair on one type is the whole point of the crate.

`crates/mercury/Cargo.toml` gains `freddie_wire`, and `external.rs` keeps `DEFAULT_PORT`, `on_message`, and its tests, re-exporting the vocabulary rather than defining it:

```rust
pub use freddie_wire::{IncomingEvent, TabMessage};
```

Nothing else in mercury changes, and its tests pin the same behavior as before.

## Change 2: the extension

`chrome-extension/Cargo.toml`:

```toml
[workspace]

[package]
name = "mercury_extension"
description = "The mercury bridge: pushes Chrome's active tab URL to mercury."
version = "0.0.1"
edition = "2024"
license = "MIT"

[lib]
crate-type = ["cdylib"]

[dependencies]
freddie_wire = { path = "../crates/freddie_wire" }
wasm-bindgen = "0.2"
serde_json = "1"

[dependencies.web-sys]
version = "0.3"
features = ["WebSocket", "MessageEvent", "Event", "BinaryType"]

[profile.release]
# The binary ships to a browser, so size is the thing to optimize.
opt-level = "z"
lto = true
```

`chrome-extension/src/lib.rs`:

```rust
//! The mercury bridge's wasm half: it owns the socket and the frame encoding.
//!
//! The JavaScript shim owns the `chrome.*` listeners, because those are callback registrations that
//! read better where they are declared, and it calls [`push_url`] with a URL and nothing else. So
//! every frame on the wire is built here, by `freddie_wire`'s own `Serialize`, and the shim never
//! constructs one.

use std::cell::RefCell;

use freddie_wire::{IncomingEvent, TabMessage};
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

thread_local! {
    /// The live socket, if there is one. A service worker is single-threaded, so a `thread_local`
    /// `RefCell` is the whole of the synchronization needed.
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
}

/// Push `url` to mercury, opening the socket if it is not already open.
///
/// `port` comes from the shim, which reads it from extension storage; this side does not know what
/// a `chrome.storage` is.
///
/// Failures are dropped rather than reported. There is nothing useful to do with a URL that could
/// not be sent: the next tab event supersedes it, and a retry queue would be a queue of stale
/// answers to a question nobody asked yet.
#[wasm_bindgen]
pub fn push_url(port: u16, url: String) {
    let frame = match serde_json::to_string(&IncomingEvent::Tab(TabMessage { url })) {
        Ok(frame) => frame,
        Err(_) => return,
    };
    SOCKET.with(|socket| {
        let mut socket = socket.borrow_mut();
        if let Some(open) = socket.as_ref().filter(|ws| ws.ready_state() == WebSocket::OPEN) {
            let _ = open.send_with_str(&frame);
            return;
        }
        // Not open: connect, and send once it is. A socket still CONNECTING gets the same
        // treatment, so a burst of tab events during a connect all go out on `open`.
        let connecting = match socket.as_ref() {
            Some(ws) if ws.ready_state() == WebSocket::CONNECTING => ws.clone(),
            _ => match connect(port) {
                Some(ws) => ws,
                None => return,
            },
        };
        let on_open = Closure::once_into_js({
            let ws = connecting.clone();
            move || {
                let _ = ws.send_with_str(&frame);
            }
        });
        connecting.add_event_listener_with_callback("open", on_open.unchecked_ref()).ok();
        *socket = Some(connecting);
    });
}

/// Open a socket to mercury, clearing [`SOCKET`] when it closes so the next push reconnects.
///
/// Each handler clears only its own socket. A connection that fails fires `error` and then `close`,
/// and by then a later push may already have replaced it; clearing unconditionally would drop a
/// live socket and strand whatever it was about to send.
fn connect(port: u16) -> Option<WebSocket> {
    let ws = WebSocket::new(&format!("ws://127.0.0.1:{port}")).ok()?;
    for event in ["close", "error"] {
        let forget = Closure::<dyn FnMut()>::new({
            let ws = ws.clone();
            move || {
                SOCKET.with(|socket| {
                    let mut socket = socket.borrow_mut();
                    if socket.as_ref().is_some_and(|live| live == &ws) {
                        *socket = None;
                    }
                });
            }
        });
        ws.add_event_listener_with_callback(event, forget.as_ref().unchecked_ref()).ok()?;
        forget.forget();
    }
    Some(ws)
}
```

`chrome-extension/js/background.js`, the shim:

```js
// Registers the chrome listeners and hands URLs to wasm. It builds no frames and knows no wire
// format: `push_url` is the whole interface.

import init, { push_url } from "../pkg/mercury_extension.js";

// mercury's default, from `freddie_wire`'s side of the contract. The options page overrides it, and
// it has to match whatever `--port` or `MERCURY_PORT` mercury was given.
const DEFAULT_PORT = 3883;

// The wasm module is instantiated once per service-worker lifetime. The worker is killed after
// roughly 30s idle and revived by these listeners, so this runs again on the next tab event.
const ready = init();

async function pushUrl(url) {
  if (!url) return;
  await ready;
  const { port } = await chrome.storage.local.get({ port: DEFAULT_PORT });
  push_url(port, url);
}

chrome.tabs.onActivated.addListener(({ tabId }) => {
  chrome.tabs.get(tabId, (tab) => pushUrl(tab.url));
});

chrome.tabs.onUpdated.addListener((_tabId, info, tab) => {
  if (info.url && tab.active) pushUrl(tab.url);
});

// Returning from another app, and switching between two Chrome windows, both change which tab is
// front without any tab event firing. WINDOW_ID_NONE means Chrome lost focus, so there is nothing
// to report; otherwise the active tab of the window that just got focus is the answer.
chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) return;
  chrome.tabs.query({ active: true, windowId }, ([tab]) => pushUrl(tab?.url));
});
```

`chrome-extension/manifest.json`:

```json
{
  "manifest_version": 3,
  "name": "mercury bridge",
  "version": "0.0.1",
  "background": { "service_worker": "js/background.js", "type": "module" },
  "permissions": ["tabs", "storage"],
  "host_permissions": ["http://127.0.0.1/*"],
  "options_page": "options.html",
  "content_security_policy": {
    "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'"
  }
}
```

`"type": "module"` is what lets the shim `import` the wasm-bindgen glue. `wasm-unsafe-eval` is what lets the extension instantiate its own wasm at all; MV3 refuses `WebAssembly.instantiate` under the default policy, and this keyword is the sanctioned way to allow it without loosening anything about scripts.

`tabs` grants the `url` field on a `Tab`. `host_permissions` for the loopback lets the worker open the WebSocket: Chrome checks a `ws://` connection against the matching `http://` host permission, and match patterns carry no port, so `http://127.0.0.1/*` covers every port mercury might be on.

`chrome-extension/options.html` and `js/options.js` are one number input over `chrome.storage.local.port`, defaulting to `DEFAULT_PORT`. mercury's port moves with `--port` or `MERCURY_PORT`, and an extension pinned to one number would connect to nothing the moment it did, with no symptom beyond per-site binds quietly not working. Storage is read on every push rather than cached, so a change takes effect on the next tab event.

## Change 3: the README

`chrome-extension/README.md`, because the build step means this cannot be loaded straight from a checkout:

````markdown
# mercury bridge

Pushes Chrome's active tab URL to a running mercury, which is how per-site key remaps know what
site you are on. Rust compiled to wasm, so the frames it sends are built by the same types mercury
parses them with (`crates/freddie_wire`).

## Build

Once:

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
```

Then, from this directory:

```
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/mercury_extension.wasm
```

`--target web` is the one that works in an MV3 service worker: it emits an ES module with an `init`
you call yourself, and no bundler.

## Install

1. Build, above. `pkg/` has to exist before Chrome will load this.
2. Open `chrome://extensions` and turn on Developer mode.
3. Load unpacked, and choose this directory.
4. If mercury is not on port 3883, open the extension's Details, then Extension options, and set
   the port to match whatever `--port` or `MERCURY_PORT` it was given.

Rebuilding does not reload the extension. Press the reload arrow on its card at `chrome://extensions`
after every build.

## Check that it works

With mercury running:

```
tail -f ~/Library/Logs/mercury/mercury.log
```

Switch tabs. Each switch should log a dispatch whose state carries the new URL:

```
Foreground { app: Chrome(ForegroundedChrome { url: Some("https://claude.ai/new") }), ... }
```

Nothing at all means the socket never opened: check that mercury is running, that the port matches,
and the service worker's own console, reachable from the extension's card at `chrome://extensions`.
````

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket and
drops the wasm instance with it. This is why there is no connection to hold and no timer to keep one
warm: the worker's registered listeners revive it when a tab event fires, `init()` runs again, and
the first push reconnects. A dead socket while idle is correct, and it costs one instantiation and
one reconnect on the next real event.

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.

## To confirm while building this

Every one of these is cheap to settle once the extension loads, and none of them is settled now.

- That `wasm-bindgen --target web` output instantiates inside an MV3 service worker. The glue resolves the `.wasm` relative to `import.meta.url` and fetches it, which is same-origin under `chrome-extension://` and should be allowed, but "should" is doing work in that sentence.
- That `wasm-unsafe-eval` in `extension_pages` covers the service worker, and not only extension pages proper.
- That `chrome.windows.onFocusChanged` fires when focus returns from another application, rather than only between Chrome windows, and whether devtools taking focus produces a spurious push.
- Whether `onUpdated` carries `info.url` for same-document navigations. It is the event for document loads; `chrome.webNavigation.onHistoryStateUpdated` and `onReferenceFragmentUpdated` exist for History API and fragment changes. This costs nothing while `Site::from_url` matches on the host, since an SPA route change inside `claude.ai` leaves the host alone, and it starts mattering the moment a bind keys on the path.
