# external events: a socket mercury can be told things on

Not built. There is exactly one mercury (`refactors/past/single-instance.md`), and it listens on a loopback WebSocket so processes outside it can push events in. This doc owns that direction only: what mercury may be told, and how a frame becomes a `MercuryEvent`.

Mercury asking the outside world for something is the other direction, and it is `external-effects.md`. Nothing here sends anything down the socket.

The Chrome extension is the first client (`chrome-extension.md`), pushing the frontmost tab's URL. A CLI that pokes mercury, a voice mode, and a test harness that drives the model without a keyboard are all the same client from mercury's side and speak the same vocabulary.

## The mechanism

`run_event_loop` reads `MercuryEvent`s off `event_tx` and does not care where they came from. Today's senders are the keyboard tap, the `freddie_app_nav` watcher, the menu bar, and the timer tasks. A socket is one more sender: a task owns a listener, accepts connections, deserializes each text frame, and sends the resulting event on the same channel.

The transport crate stays dumb and mercury owns the vocabulary, the same split `freddie_app_nav` uses (it hands up a bundle-id string; `App::from_bundle_id` decides what that string means). `freddie_event_socket` hands up frames as `&str`; `crates/mercury/src/external.rs` maps each to an event.

## `IncomingEvent` is the whole vocabulary, and v0 ships it empty

`MercuryEvent` does not derive `Deserialize`, and it should not. Deriving it makes `MercuryEvent::Key` and `MercuryEvent::Quit` constructible from the wire, so "no remote keyboard, no remote kill" becomes a rule some match arm enforces rather than something the types say. `IncomingEvent` names exactly what an outside sender may say, and there is no arm to forget.

```rust
// crates/mercury/src/external.rs

/// Everything an outside process may say to mercury. A sender cannot say anything else, so remote
/// key injection and remote quit are unrepresentable rather than filtered.
///
/// Empty, so every frame is refused with `unknown variant `IncomingEvent.Tab`, there are no
/// variants`. This doc ships the transport, and an empty vocabulary is the honest statement of what
/// mercury can be told once it exists. Variants arrive with the features that need them.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {}
```

Verified against the pinned toolchain: an adjacently tagged enum with no variants derives, compiles, and errors on every input.

`chrome-tab-url.md`'s `TabEvent` adds the first variant:

```rust
 pub enum IncomingEvent {
+    /// The frontmost browser tab's URL changed.
+    #[serde(rename = "IncomingEvent.Tab")]
+    Tab(TabMessage),
 }

+#[derive(serde::Deserialize, Debug)]
+pub struct TabMessage {
+    pub url: String,
+}
```

Every message is the project's one discriminated-union form, `{ kind: "Type.Variant", value: T }`, which is what adjacent tagging (`tag = "kind", content = "value"`) over newtype variants produces:

```jsonc
// client -> mercury
{ "kind": "IncomingEvent.Tab", "value": { "url": "https://claude.ai/new" } }
```

## The endpoint

`127.0.0.1:8797`, overridable with `MERCURY_PORT`, which is the same shape as `LOG_LEVEL` in `crates/mercury/src/logging.rs`: an environment variable read at startup, no config file, because mercury has no config file to put it in.

```rust
// crates/mercury/src/external.rs

/// The port mercury listens on when `MERCURY_PORT` says nothing. Hardcoded in the extension too.
///
/// Mercury's orbital period, 87.969 days, truncated to fit a `u16`. Below 49152, which is where
/// macOS starts handing out ephemeral ports (`net.inet.ip.portrange.first`): a listener up there
/// can find its port already taken by some outbound socket that grabbed it first. Unassigned in
/// `/etc/services`.
pub const DEFAULT_PORT: u16 = 8797;

const PORT_ENV: &str = "MERCURY_PORT";

/// The port to listen on.
///
/// A `MERCURY_PORT` that is not a `u16` panics. Falling back to the default would leave mercury
/// listening somewhere the extension is not, which presents as "per-site binds stopped working"
/// and sends you looking at the extension. The typo is in a shell profile; say so and stop.
///
/// Called from `main` before the single-instance lock and before the keyboard grab, so the panic
/// costs a process that has not touched the machine yet.
#[must_use]
pub fn port() -> u16 {
    let Some(raw) = std::env::var_os(PORT_ENV) else {
        return DEFAULT_PORT;
    };
    raw.to_str()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("MERCURY_PORT is {raw:?}, which is not a port number"))
}
```

Loopback only, never `0.0.0.0`: a wildcard bind would let anything on the network tell mercury which app you are looking at.

A failed bind is fatal. A mercury that came up without its socket is a mercury whose per-site binds silently do nothing, and there is no way to tell that apart from a broken extension without reading the log; there should be no "running, but deaf" mercury to be in. The single-instance lock already means the squatter is some other program, so the message names the port and says how to find it.

## No token, but web pages are refused

Anything that can reach this socket can tell mercury which URL is frontmost, and nothing else. The worst a hostile client achieves is a lie about the current tab, which makes a site-specific key send a chord to the wrong page, and only on the next press of that key. That is a nuisance, and it costs less than a shared secret the user has to paste in. `external-effects.md` is where this stops being true, and it is the doc that introduces the token.

A web page is a different matter, and it is closed here rather than left to the token. WebSockets are exempt from the same-origin policy: the handshake is allowed to cross origins, the browser attaches an `Origin` header, and the server decides. So any page in any open tab can `new WebSocket("ws://127.0.0.1:8797")` and start talking. Chrome's Private Network Access work aims at this, but its WebSocket enforcement has been partial and has moved around, so nothing here depends on it.

The rule is a denylist, not an allowlist:

- An `Origin` of `http://` or `https://` is a web page. Refuse it.
- No `Origin` at all is a native client (a CLI, `websocat`, the test harness). Allow it.
- A `chrome-extension://` origin is the extension. Allow it.

An allowlist that demanded `chrome-extension://` would lock out the CLI and the test harness, which are clients this doc exists to serve. The denylist costs those nothing and still removes every page on the web from the set of things that can reach mercury.

The extension's id is not pinned. An unpacked development build's id depends on where it is loaded from and differs from a packed one, so pinning it would break the install path in `chrome-extension.md` for no gain: the remaining set is "other extensions the user chose to install," which is not the set the check is for.

Confirm at implementation time that a Chrome MV3 service worker's `WebSocket` actually sends `Origin: chrome-extension://<id>`, and drop to "absent `Origin`" as the extension's case if it does not. The denylist works either way; only the third bullet depends on it.

## `freddie_event_socket`

Its own crate, in the mold of `freddie_app_nav`: it owns the listener, runs its accept and read loops off the main path, calls a callback per frame, and closes everything when dropped. It knows nothing about `MercuryEvent`, so figaro can take an event socket by calling the same `listen`.

```rust
// crates/freddie_event_socket/src/lib.rs

/// The listener. Dropping it stops accepting and closes every live connection.
pub struct EventSocket { /* the accept task's `JoinHandle`, aborted on drop */ }

/// Bind `127.0.0.1:port` and call `on_message` for each text frame any client sends.
///
/// `on_message` runs on the socket's runtime, so it must not block: send on a channel and return,
/// the way `freddie_app_nav::watch`'s callback does.
pub fn listen<F>(port: u16, on_message: F) -> std::io::Result<EventSocket>
where
    F: Fn(&str) + Send + Sync + 'static;

/// Whether a handshake carrying this `Origin` may connect.
///
/// A browser attaches `Origin` to a WebSocket handshake and leaves the decision to the server,
/// which is the whole of what stops a page in an open tab from driving the socket. Native clients
/// send none, so absent is allowed; a page's `http`/`https` origin is not.
fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => !o.starts_with("http://") && !o.starts_with("https://"),
    }
}
```

`tokio-tungstenite` is the transport, on the runtime mercury already has. Per accepted connection the crate spawns a read task, and:

- The handshake is refused with 403 when `origin_allowed` says no, before any frame is read.
- Only text frames reach `on_message`. A binary frame is dropped with a `debug` line; pings are answered by tungstenite itself.
- `max_message_size` is 64 KiB. A URL is small, and nothing that arrives here should be able to make mercury allocate without bound.
- Concurrent connections are fine and expected (the extension and a CLI at once). Every connection's frames land on the same `on_message`.

## Change 1: the crate

New `crates/freddie_event_socket`, workspace member, `tokio` and `tokio-tungstenite`, no mercury types. It ships alone: nothing in mercury calls it yet.

Tests, driven by a `tokio-tungstenite` client against a listener on port 0:

- A text frame arrives at `on_message` intact.
- Two concurrent connections both deliver.
- A handshake with `Origin: https://evil.com` is refused, one with `Origin: http://localhost:3000` is refused too (a page served from loopback is still a page), and one with `chrome-extension://abc` or no `Origin` connects.
- A binary frame does not reach `on_message`, and the connection stays open.
- A frame over `max_message_size` closes that connection and leaves the listener accepting.
- Dropping the `EventSocket` closes live connections and frees the port for an immediate rebind.

## Change 2: mercury listens

`crates/mercury/Cargo.toml` gains:

```toml
freddie_event_socket = { path = "../freddie_event_socket", version = "0.0.1" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`crates/mercury/src/external.rs` is new and holds `IncomingEvent`, `DEFAULT_PORT`, `port`, and:

```rust
/// Turn one frame into an event and send it. Runs on the socket's runtime, so it parses, sends,
/// and returns.
///
/// A frame that is not valid `IncomingEvent` is logged and dropped: a client speaking nonsense is a
/// client bug, not a reason to tear the connection down. With `IncomingEvent` empty that is every frame,
/// and connecting and being ignored is exactly what a client should see here.
pub fn on_message(text: &str) {
    match serde_json::from_str::<IncomingEvent>(text) {
        // `IncomingEvent` has no variants, so this value cannot exist and the arm is empty. The first
        // variant to land breaks this line, which is how the compiler asks where the event goes; it
        // takes an `event_tx` from then on.
        Ok(never) => match never {},
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

`crates/mercury/src/main.rs`, in `run`, above `freddie_keyboard::intercept` so a refused start has not grabbed the keyboard yet:

```rust
    // The external event source. Held for the length of `run`, like `_watcher`: dropping it closes
    // the port. A busy port panics: the single-instance lock already means the squatter is some
    // other program, and a mercury that came up deaf presents as "the extension broke" while
    // looking perfectly healthy.
    let _socket = freddie_event_socket::listen(port, |text| mercury::on_message(text))
        .unwrap_or_else(|e| {
            panic!("could not bind 127.0.0.1:{port}: {e}; find it with `lsof -i :{port}`")
        });
```

`port` comes from `main`, which calls `mercury::port()` before the single-instance lock, so a bad `MERCURY_PORT` panics before the process has touched anything.

Verify by hand with any WebSocket client: connect to `ws://127.0.0.1:8797`, send `{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}`, and watch `~/Library/Logs/mercury/mercury.log` record the refusal naming the unknown variant. Once `chrome-tab-url.md` has added `IncomingEvent::Tab`, the same frame produces a dispatch record instead.
