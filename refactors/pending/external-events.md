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
/// variants`. This doc ships the transport, and until a feature needs to be told something there
/// is nothing mercury can be told. Variants arrive with the features that want them.
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

`127.0.0.1:8797`, overridable by flag or environment variable, flag first:

```
cargo run -p mercury -- --port 9000
MERCURY_PORT=9000 cargo run -p mercury
```

`mercury-cli.md` owns that: it puts clap in front of mercury and gives `Args` a `port` field whose one `#[arg(long, env = "MERCURY_PORT", default_value_t = DEFAULT_PORT)]` declares the flag, the variable, the default, and the precedence between them. A value that is not a `u16` exits with clap's message before `main` does anything. Nothing here parses arguments.

The flag matters as much as the variable: running a second mercury on another port is a one-off, and exporting a variable for it would follow every later run in that terminal.

What stays here is the number and why it is that number:

```rust
// crates/mercury/src/external.rs

/// The port mercury listens on when nothing overrides it. Hardcoded in the extension too.
///
/// Mercury's orbital period, 87.969 days, truncated to fit a `u16`. Below 49152, which is where
/// macOS starts handing out ephemeral ports (`net.inet.ip.portrange.first`): a listener up there
/// can find its port already taken by some outbound socket that grabbed it first. Unassigned in
/// `/etc/services`.
pub const DEFAULT_PORT: u16 = 8797;
```

Overriding it means editing `MERCURY_URL` in the extension's `background.js` to match, since the extension hardcodes the same number.

Loopback only, never `0.0.0.0`: a wildcard bind would let anything on the network tell mercury which app you are looking at.

A failed bind is fatal. A mercury that came up without its socket is a mercury whose per-site binds silently do nothing, and there is no way to tell that apart from a broken extension without reading the log; there should be no "running, but deaf" mercury to be in. The single-instance lock already means the squatter is some other program, so the message names the port and says how to find it.

## The origin check

Mercury asks for no token here. A client that reaches this socket can report which URL is frontmost and nothing else, so a hostile one gets to lie about the current tab, and the lie buys it one wrong chord the next time you press a site-bound key. A secret the user has to paste into an options page costs more than that is worth. `external-effects.md` adds the token, once a connection is worth something.

A web page can reach the socket, and that one gets closed. WebSockets are exempt from the same-origin policy: the handshake crosses origins freely, the browser attaches an `Origin` header, and the server decides what to do with it. Any page in any open tab can run `new WebSocket("ws://127.0.0.1:8797")` and start sending frames. Chrome's Private Network Access work aims at this case, but its WebSocket enforcement has been partial and has shifted between releases, so the check assumes nothing from it.

Three origins arrive, and the first is refused with a 403 at the handshake:

- `http://` or `https://`, which is a web page.
- absent, which is a native client: a CLI, `websocat`, the test harness.
- `chrome-extension://`, which is the extension.

Demanding `chrome-extension://` instead would lock out the CLI and the test harness, and this doc exists to serve those too. Refusing the web schemes leaves them alone and still puts every page on the web out of reach.

Any `chrome-extension://` origin is accepted, whatever the id. An unpacked development build's id follows from where it was loaded and a packed build's differs again, so matching one id would break the install path `chrome-extension.md` describes, and what it would exclude is other extensions the user installed deliberately.

Confirm during implementation that a Chrome MV3 service worker's `WebSocket` sends `Origin: chrome-extension://<id>`. If it sends none, the extension arrives as the absent case and the check still holds.

## `freddie_event_socket`

Its own crate, in the mold of `freddie_app_nav`: it owns the listener, runs its accept and read loops off the main path, calls a callback per frame, and closes everything when dropped. It knows nothing about `MercuryEvent`, so figaro can take an event socket by calling the same `listen`.

## Change 1: the crate

New `crates/freddie_event_socket`. It ships alone: nothing in mercury calls it yet.

`Cargo.toml` at the workspace root, before:

```toml
    "freddie_app_nav",
    "freddie_main_loop",
```

after:

```toml
    "freddie_app_nav",
    "freddie_event_socket",
    "freddie_main_loop",
```

`crates/freddie_event_socket/Cargo.toml`:

```toml
[package]
name = "freddie_event_socket"
description = "A loopback WebSocket that hands each text frame to a callback."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
# 0.24 pins tungstenite 0.24, where `WebSocketConfig` is a `#[non_exhaustive]` struct with public
# fields. 0.26 replaced those with builder methods, so the config literal below changes with it.
tokio-tungstenite = "0.24"
tokio = { version = "1", features = ["net", "rt", "macros", "sync"] }
futures-util = { version = "0.3", default-features = false }
http = "1"
tracing = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["net", "rt", "macros", "sync", "time"] }

[lints]
workspace = true
```

`crates/freddie_event_socket/src/lib.rs`:

```rust
//! A loopback WebSocket that hands each text frame to a callback.
//!
//! A source in the mold of `freddie_app_nav`: [`listen`] binds the port and calls back per frame,
//! the caller decides what a frame means, and dropping the returned [`EventSocket`] closes
//! everything. It knows nothing about any particular event type.
//!
//! Web pages are refused at the handshake. A browser attaches `Origin` to a WebSocket handshake and
//! leaves the decision to the server, and WebSockets are exempt from the same-origin policy, so
//! without this check any page in any open tab could drive the socket.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::{debug, warn};

/// A frame past this closes the connection that sent it. Nothing that belongs on this socket is
/// large, and a client must not be able to make the process allocate without bound.
const MAX_FRAME_BYTES: usize = 64 * 1024;

/// The listener. Dropping it stops accepting and closes every live connection.
///
/// It owns the only `watch::Sender`, and every task holds a receiver, so the drop is what each of
/// them is waiting on. There is nothing to abort by hand.
pub struct EventSocket {
    _shutdown: watch::Sender<()>,
}

/// Bind `127.0.0.1:port` and call `on_message` for each text frame any client sends.
///
/// `on_message` runs on the socket's runtime, so it must not block: send on a channel and return,
/// the way `freddie_app_nav::watch`'s callback does. It is called from every connection, so several
/// clients share one callback.
///
/// The bind is synchronous, through `std`, so a busy port is an `Err` from this call rather than a
/// failure inside a spawned task that the caller would have to go looking for.
///
/// # Errors
///
/// If the port is taken, or loopback cannot be bound.
///
/// # Panics
///
/// If called outside a tokio runtime.
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    let on_message = Arc::new(on_message);

    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = dropped(&mut closed) => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted");
                    tokio::spawn(serve(stream, Arc::clone(&on_message), closed.clone()));
                }
                // A refused connection is that client's problem; the listener keeps accepting.
                Err(e) => debug!(error = %e, "accept failed"),
            }
        }
        debug!("the event socket closed");
    });

    Ok(EventSocket { _shutdown: shutdown })
}

/// Resolves once the [`EventSocket`] has been dropped, taking the only sender with it.
async fn dropped(closed: &mut watch::Receiver<()>) {
    while closed.changed().await.is_ok() {}
}

/// One connection: handshake, then every text frame to `on_message` until it ends or the socket is
/// dropped.
async fn serve<F>(stream: TcpStream, on_message: Arc<F>, mut closed: watch::Receiver<()>)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let config = WebSocketConfig {
        max_message_size: Some(MAX_FRAME_BYTES),
        ..WebSocketConfig::default()
    };
    let mut ws = match accept_hdr_async_with_config(stream, check_origin, Some(config)).await {
        Ok(ws) => ws,
        Err(e) => {
            debug!(error = %e, "handshake failed");
            return;
        }
    };

    loop {
        let frame = tokio::select! {
            () = dropped(&mut closed) => break,
            frame = ws.next() => frame,
        };
        match frame {
            Some(Ok(Message::Text(text))) => on_message(text.as_str()),
            Some(Ok(Message::Binary(_))) => debug!("dropping a binary frame"),
            // Ping and Close are tungstenite's to answer, and it has already queued the reply.
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                debug!(error = %e, "connection ended");
                break;
            }
            None => break,
        }
    }
}

/// The handshake gate: refuse a web page, admit everything else.
fn check_origin(request: &Request, response: Response) -> Result<Response, ErrorResponse> {
    let origin = match request.headers().get(http::header::ORIGIN) {
        None => None,
        Some(value) => match value.to_str() {
            Ok(origin) => Some(origin),
            // An origin that is not text is one this cannot clear, so it does not get to.
            Err(_) => return Err(refuse()),
        },
    };
    if origin_allowed(origin) {
        Ok(response)
    } else {
        warn!(?origin, "refusing a handshake from a web page");
        Err(refuse())
    }
}

fn refuse() -> ErrorResponse {
    let mut response = ErrorResponse::new(Some("origin not allowed".to_owned()));
    *response.status_mut() = http::StatusCode::FORBIDDEN;
    response
}

/// Whether a handshake carrying this `Origin` may connect.
///
/// Native clients send none, so absent connects. A page's `http`/`https` origin does not, and that
/// includes a page served from loopback, which is still a page. Anything else, in practice
/// `chrome-extension://<id>`, connects; the id is not matched, because an unpacked development
/// build's id follows from where it was loaded and a packed build's differs again.
fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(origin) => !origin.starts_with("http://") && !origin.starts_with("https://"),
    }
}
```

The `lib.rs` above was compiled against `tokio-tungstenite` 0.24 and driven with a real `connect_async` client before this doc was written: it is clippy-clean under the workspace's `pedantic` and `nursery` lints, and every behavior listed below was observed rather than assumed. `Message::Text` carries a `Utf8Bytes` in 0.24, and `text.as_str()` works on both that and the `String` earlier versions used.

`crates/freddie_event_socket/tests/socket.rs` drives a real client against a listener on port 0, reading back the bound port with `TcpListener::local_addr` so concurrent test binaries never collide:

- `origin_allowed` directly: `None`, `chrome-extension://abc` allowed; `https://evil.com`, `http://localhost:3000` refused. It is the one piece of logic here that is a pure function, so it gets a table rather than a socket.
- A text frame arrives at `on_message` intact, including one with multibyte UTF-8.
- Two concurrent connections both deliver, and one closing leaves the other delivering.
- A handshake with `Origin: https://evil.com` is refused with 403 and `on_message` never runs.
- A binary frame does not reach `on_message`, and the next text frame on that same connection still does.
- A frame over `MAX_FRAME_BYTES` closes that connection, and a fresh connection still works.
- Dropping the `EventSocket` closes a live connection, seen by the client's next read ending, and frees the port for an immediate rebind on the same number.

## Change 2: mercury listens

Depends on `mercury-cli.md`, which puts clap in front of mercury. `Args` gains the port here, before:

```rust
pub struct Args {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,
}
```

after:

```rust
pub struct Args {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}
```

`u16` is the whole of the validation, and clap does it: confirmed against clap 4, `--port abc` exits with `invalid value 'abc' for '--port <PORT>': invalid digit found in string` and `--port 99999` with `99999 is not in 0..=65535`. A bad `MERCURY_PORT` gets that same message, naming `--port` rather than the variable that carried the value; that is clap's wording, and it is what you will see when the typo is in a shell profile.

`crates/mercury/Cargo.toml`, before:

```toml
freddie_app_nav = { path = "../freddie_app_nav", version = "0.0.1" }
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
```

after:

```toml
freddie_app_nav = { path = "../freddie_app_nav", version = "0.0.1" }
freddie_event_socket = { path = "../freddie_event_socket", version = "0.0.1" }
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
```

and, with the other non-freddie dependencies:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`crates/mercury/src/lib.rs`, before:

```rust
mod effect;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{MercuryEffect, Placement};
```

after:

```rust
mod effect;
mod external;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{MercuryEffect, Placement};
pub use external::{DEFAULT_PORT, on_message};
```

`crates/mercury/src/external.rs` is new and holds `IncomingEvent`, `DEFAULT_PORT`, and:

```rust
/// Turn one frame into an event and send it. Runs on the socket's runtime, so it parses, sends,
/// and returns.
///
/// A frame that is not valid `IncomingEvent` is logged and dropped: a client speaking nonsense is a
/// client bug, not a reason to tear the connection down. With `IncomingEvent` empty that is every
/// frame, and connecting and being ignored is exactly what a client should see here.
pub fn on_message(text: &str) {
    match serde_json::from_str::<IncomingEvent>(text) {
        // `IncomingEvent` has no variants, so this value cannot exist and the arm is empty. The
        // first variant to land breaks this line, which is how the compiler asks where the event
        // goes; it takes an `event_tx` from then on.
        Ok(never) => match never {},
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

`crates/mercury/src/main.rs`. `mercury-cli.md` has already parsed `Args` at the top of `main`, before the lock and the keyboard, so a bad `--port` has exited by this point. `run` takes the port from it. Before:

```rust
            runtime.block_on(run(event_tx, event_rx, title_tx));
```

after:

```rust
            runtime.block_on(run(event_tx, event_rx, title_tx, args.port));
```

Before:

```rust
async fn run(
    event_tx: UnboundedSender<MercuryEvent>,
    event_rx: UnboundedReceiver<MercuryEvent>,
    title_tx: std::sync::mpsc::Sender<&'static str>,
) {
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // Grab the keyboard: swallow every key and forward it to the model, which
    // decides what to emit (the effect loop performs it).
    let grabbed = freddie_keyboard::intercept({
```

after:

```rust
async fn run(
    event_tx: UnboundedSender<MercuryEvent>,
    event_rx: UnboundedReceiver<MercuryEvent>,
    title_tx: std::sync::mpsc::Sender<&'static str>,
    port: u16,
) {
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // The external event source. Held for the length of `run`, like `_watcher`: dropping it closes
    // the port. Above the keyboard grab, so a refused start has not taken the keyboard yet.
    //
    // A busy port panics. The single-instance lock already means the squatter is some other
    // program, and a mercury that came up deaf presents as "the extension broke" while looking
    // perfectly healthy.
    let _socket = freddie_event_socket::listen(port, mercury::on_message).unwrap_or_else(|e| {
        panic!("could not bind 127.0.0.1:{port}: {e}; find it with `lsof -i :{port}`")
    });

    // Grab the keyboard: swallow every key and forward it to the model, which
    // decides what to emit (the effect loop performs it).
    let grabbed = freddie_keyboard::intercept({
```

`mercury::on_message` is passed as a function item rather than a closure, since it captures nothing until `IncomingEvent` has a variant.

Verify by hand. `cargo run -p mercury`, then from another pane:

```
websocat ws://127.0.0.1:8797
{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}
```

`~/Library/Logs/mercury/mercury.log` records the refusal naming the unknown variant, and the connection stays open. Then check the gate and the overrides:

- `websocat -H='Origin: https://evil.com' ws://127.0.0.1:8797` is refused with 403.
- `cargo run -p mercury -- --port 9000` and `MERCURY_PORT=9000 cargo run -p mercury` both listen on 9000, and `--port 9000` wins over `MERCURY_PORT=9001`.
- `cargo run -p mercury -- --port abc` and `--prot 9000` both exit with clap's message before the menu-bar icon appears.
- A second `cargo run -p mercury` on the same port is refused by the single-instance lock, and one started with `--port 9000` while another holds 8797 is refused by the lock too.

Once `chrome-tab-url.md` has added `IncomingEvent::Tab`, the same frame produces a dispatch record instead of a refusal.
