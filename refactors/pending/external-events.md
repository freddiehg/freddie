# mercury's external event socket

Not built. There is exactly one mercury (`refactors/past/single-instance.md`), and it listens on a loopback WebSocket so processes outside it can push events in. The Chrome extension is the first client (`chrome-extension.md`), and this doc owns everything on mercury's side of that wire: the port, the transport crate, the message vocabulary, how a message becomes a `MercuryEvent`, the command channel back down, and the token.

The extension is not the only client this serves. A CLI that pokes mercury, a voice mode, and a test harness that drives the model without a keyboard all connect the same way and speak the same vocabulary.

## The mechanism

`run_event_loop` reads `MercuryEvent`s off `event_tx` and does not care where they came from. Today's senders are the keyboard tap, the `freddie_app_nav` watcher, the menu bar, and the timer tasks. A socket is one more sender: a task owns a listener, accepts connections, deserializes each text frame into a `MercuryEvent`, and sends it on the same channel.

The transport crate stays dumb and mercury owns the vocabulary, the same split `freddie_app_nav` uses (it hands up a bundle-id string; `App::from_bundle_id` decides what that string means). `freddie_event_socket` hands up frames as `&str`; `crates/mercury/src/external.rs` maps each to an event.

## The external vocabulary is its own type

`MercuryEvent` does not derive `Deserialize`, and it should not. Deriving it makes `MercuryEvent::Key` and `MercuryEvent::Quit` constructible from the wire, so "no remote keyboard, no remote kill" becomes a rule some match arm enforces rather than something the types say. A separate `Upstream` enum names exactly what an outside sender may say, and there is no arm to forget.

```rust
// crates/mercury/src/external.rs

/// A message an outside process may send mercury. This is the whole external vocabulary: a
/// sender cannot say anything else, so remote key injection and remote quit are unrepresentable
/// rather than filtered.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Upstream {
    /// The frontmost browser tab's URL changed.
    #[serde(rename = "Upstream.Tab")]
    Tab(TabMessage),
    /// A reply to a command mercury sent down. Consumed by the command bus, never dispatched.
    #[serde(rename = "Upstream.Reply")]
    Reply(ReplyMessage),
}

#[derive(serde::Deserialize, Debug)]
pub struct TabMessage {
    pub url: String,
}

#[derive(serde::Deserialize, Debug)]
pub struct ReplyMessage {
    pub id: CommandId,
    pub result: Result,
}

/// A message mercury sends down to a connected client.
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Downstream {
    #[serde(rename = "Downstream.Command")]
    Command(CommandMessage),
}

#[derive(serde::Serialize, Debug)]
pub struct CommandMessage {
    pub id: CommandId,
    pub command: Command,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Command {
    #[serde(rename = "Command.RunJs")]
    RunJs(RunJs),
    #[serde(rename = "Command.OpenTab")]
    OpenTab(OpenTab),
    #[serde(rename = "Command.CloseTabs")]
    CloseTabs(CloseTabs),
    #[serde(rename = "Command.ReadSelection")]
    ReadSelection,
}

#[derive(serde::Serialize, Debug)]
pub struct RunJs {
    pub code: String,
}

#[derive(serde::Serialize, Debug)]
pub struct OpenTab {
    pub url: String,
}

#[derive(serde::Serialize, Debug)]
pub struct CloseTabs {
    /// A Chrome URL match pattern, as `chrome.tabs.query({ url })` takes.
    pub url_glob: String,
}

/// What a command produced.
///
/// It shadows the prelude's `Result` in this module, which is the point: it is a result, with the
/// same `Ok(value)` / `Err(message)` meaning, and the union convention makes the wire tag the type
/// name, so shadowing is what gets `Result.Ok` / `Result.Err` on the wire. Anything in this module
/// that wants the prelude's spells it `std::result::Result`.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Result {
    /// Whatever JSON the handler returned.
    #[serde(rename = "Result.Ok")]
    Ok(serde_json::Value),
    /// The message from a command that threw.
    #[serde(rename = "Result.Err")]
    Err(String),
}
```

Every message is the project's one discriminated-union form, `{ kind: "Type.Variant", value: T }`, so the JSON on the wire reads:

```jsonc
// client -> mercury
{ "kind": "Upstream.Tab", "value": { "url": "https://claude.ai/new" } }
{ "kind": "Upstream.Reply", "value": { "id": 42, "result": { "kind": "Result.Ok", "value": "hello" } } }
{ "kind": "Upstream.Reply", "value": { "id": 42, "result": { "kind": "Result.Err", "value": "no active tab" } } }

// mercury -> client
{ "kind": "Downstream.Command", "value": { "id": 42, "command": { "kind": "Command.RunJs", "value": { "code": "document.title" } } } }
{ "kind": "Downstream.Command", "value": { "id": 43, "command": { "kind": "Command.ReadSelection" } } }
```

Adjacent tagging (`tag = "kind", content = "value"`) over newtype variants produces exactly this, and a unit variant like `Command.ReadSelection` omits `value`.

## The endpoint

`127.0.0.1:48291`, overridable with `MERCURY_PORT`, which is the same shape as `LOG_LEVEL` in `crates/mercury/src/logging.rs`: an environment variable read at startup, no config file, because mercury has no config file to put it in.

```rust
// crates/mercury/src/external.rs

/// The port mercury listens on when `MERCURY_PORT` says nothing. Hardcoded in the extension too.
///
/// Below 49152, which is where macOS starts handing out ephemeral ports
/// (`net.inet.ip.portrange.first`): a listener up there can find its port already taken by some
/// outbound socket that grabbed it first. Unassigned in `/etc/services`.
pub const DEFAULT_PORT: u16 = 48291;

const PORT_ENV: &str = "MERCURY_PORT";

/// The port to listen on. A `MERCURY_PORT` that is not a `u16` is ignored with a warning rather
/// than fatal: a typo in a shell profile should not cost the keyboard.
#[must_use]
pub fn port() -> u16 {
    let Some(raw) = std::env::var_os(PORT_ENV) else {
        return DEFAULT_PORT;
    };
    match raw.to_str().and_then(|s| s.parse::<u16>().ok()) {
        Some(port) => port,
        None => {
            warn!(value = ?raw, "MERCURY_PORT is not a port number; using the default");
            DEFAULT_PORT
        }
    }
}
```

Loopback only, never `0.0.0.0`: binding a wildcard address would put a channel that can foreground apps and run JavaScript in your browser on the local network.

A failed bind logs and mercury runs on without the socket. The port is a feature, not the product, and the single-instance lock is what actually guarantees one mercury; a busy port means something else is squatting on it, and losing tab URLs is a far better outcome than losing the keyboard remapper.

## `freddie_event_socket`

Its own crate, in the mold of `freddie_app_nav`: it owns the listener, runs its accept and read loops off the main path, calls a callback per frame, and closes everything when dropped. It knows nothing about `MercuryEvent`, so figaro can take an event socket by calling the same `listen`.

```rust
// crates/freddie_event_socket/src/lib.rs

/// A live client connection. Cloneable and cheap: it is a sender into that connection's write
/// task, so a caller can stash one and send to it later.
#[derive(Clone)]
pub struct Client {
    outgoing: UnboundedSender<String>,
}

impl Client {
    /// Queue `text` to this client. `Err` means the connection is gone.
    pub fn send(&self, text: String) -> Result<(), Disconnected>;
}

/// The listener. Dropping it stops accepting and closes every live connection.
pub struct EventSocket { /* the accept task's `JoinHandle`, aborted on drop */ }

/// What a connecting client must present.
pub enum Auth {
    /// Anyone on loopback may connect.
    Open,
    /// The connection is refused unless its `?token=` matches.
    Token(String),
}

/// Bind `127.0.0.1:port` and call `on_message` for each text frame any client sends.
///
/// `on_message` runs on the socket's runtime, so it must not block: send on a channel and
/// return, the way `freddie_app_nav::watch`'s callback does.
pub fn listen<F>(port: u16, auth: Auth, on_message: F) -> std::io::Result<EventSocket>
where
    F: Fn(&Client, &str) + Send + Sync + 'static;
```

`tokio-tungstenite` is the transport, on the runtime mercury already has. Per accepted connection the crate spawns a read half and a write half joined by an unbounded channel, and:

- Only text frames reach `on_message`. A binary frame is dropped with a `debug` line; pings are answered by tungstenite itself.
- `max_message_size` is 1 MiB. A `Command.RunJs` result can be large; a page cannot be allowed to make mercury allocate without bound.
- The token, when `Auth::Token`, is checked during the HTTP handshake against the request URI's `token` query parameter, and a mismatch is answered with 401 and the connection dropped before any frame is read. Checking at the handshake means an unauthorized client never gets to send anything, and a query parameter is what a Chrome service worker can attach without a CSP fight.
- Concurrent connections are fine and expected (the extension and a CLI at once). Every connection's frames land on the same `on_message`.

## Change 1: the crate

New `crates/freddie_event_socket`, workspace member, `tokio` and `tokio-tungstenite`, no mercury types. It ships alone: nothing in mercury calls it yet.

Tests, driven by a `tokio-tungstenite` client against a listener on port 0:

- A text frame arrives at `on_message` intact.
- Two concurrent connections both deliver.
- `Client::send` reaches the client that sent, and a `send` after that client disconnects is `Err(Disconnected)` rather than a panic.
- `Auth::Token` accepts the matching `?token=`, and refuses a wrong one, an absent one, and one that differs only in case.
- Dropping the `EventSocket` closes live connections and frees the port for an immediate rebind.

## Change 2: mercury listens and dispatches tab URLs

Depends on `chrome-tab-url.md`'s `TabEvent { url }` and its `on_tab` at the root, which is what an `Upstream::Tab` becomes.

`crates/mercury/Cargo.toml` gains:

```toml
freddie_event_socket = { path = "../freddie_event_socket", version = "0.0.1" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`crates/mercury/src/external.rs` is new and holds the types above, `port`, and the mapping:

```rust
/// Turn one frame into an event and send it. Runs on the socket's runtime, so it parses, sends,
/// and returns.
///
/// A frame that is not valid `Upstream` is logged and dropped: a client speaking nonsense is a
/// client bug, not a reason to tear the connection down.
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<Upstream>(text) {
        Ok(Upstream::Tab(TabMessage { url })) => {
            debug!(%url, "tab");
            let _ = event_tx.send(tab(url));
        }
        Ok(Upstream::Reply(reply)) => {
            debug!(id = reply.id, "reply with no command bus; dropping");
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

`crates/mercury/src/main.rs`, in `run`, next to the app-nav watcher:

```rust
    // The external event source. Held for the length of `run`, like `_watcher`: dropping it
    // closes the port. A bind failure is not fatal; mercury without tab URLs still remaps keys.
    let _socket = match freddie_event_socket::listen(mercury::port(), Auth::Open, {
        let event_tx = event_tx.clone();
        move |_client, text| mercury::on_message(text, &event_tx)
    }) {
        Ok(socket) => Some(socket),
        Err(e) => {
            error!(error = %e, port = mercury::port(), "could not bind the event socket");
            None
        }
    };
```

Verify by hand with any WebSocket client: connect to `ws://127.0.0.1:48291`, send `{"kind":"Upstream.Tab","value":{"url":"https://claude.ai/new"}}`, and watch `~/Library/Logs/mercury/mercury.log` record the dispatch and the resulting state.

## Change 3: the token

The socket can be reached by any process on the machine, and Change 4 makes it able to run JavaScript in your browser. A tab URL from a stray local process is benign, so Change 2 ships `Auth::Open`; the command bus does not, so the token lands before it.

The token is generated once and persisted, not minted per run. A per-run token would mean re-pasting into the extension's options page after every restart, which is the kind of friction that ends with the gate turned off.

```rust
// crates/mercury/src/external.rs

/// Read the persisted token, generating and writing one on first run.
///
/// Lives beside the single-instance lock, in the platform state directory, for the same reason:
/// it must survive across runs and must not be swept out of a temp directory. 16 bytes from
/// `/dev/urandom`, hex encoded, and the file is created `0o600`.
pub fn token() -> std::io::Result<String>;
```

`main.rs` swaps `Auth::Open` for `Auth::Token(token)` and logs where the token lives so the user can copy it:

```rust
    let auth = match mercury::token() {
        Ok(token) => {
            info!(path = %mercury::token_path().display(), "event socket token");
            Auth::Token(token)
        }
        Err(e) => {
            error!(error = %e, "no token; the event socket stays closed");
            return;
        }
    };
```

A tokenless mercury does not fall back to `Auth::Open`: an unreadable state directory is a broken install, and quietly downgrading the gate is the wrong direction to fail in.

## Change 4: the command bus

The downward half. mercury sends `Downstream::Command` and gets an `Upstream::Reply` back, so a handler can ask the browser for something and use the answer. The client half is `chrome-extension.md`.

```rust
// crates/mercury/src/external.rs

/// Monotonic per-process, so a reply is never mistaken for a reply to an earlier command.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct CommandId(u64);

/// A command that got no reply in this long is dropped and its waiter told `TimedOut`. A page
/// that hangs must not leak an entry per command forever.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// The in-flight commands, and where to send the next one.
pub struct CommandBus {
    next_id: AtomicU64,
    pending: Mutex<HashMap<CommandId, oneshot::Sender<Result>>>,
    /// The client that spoke most recently. There is no separate connect callback: the extension
    /// pushes a tab URL as soon as it connects, so the newest speaker is the live browser, and a
    /// client that never speaks is one nothing can be asked of anyway.
    target: Mutex<Option<Client>>,
}

pub enum CommandError {
    /// Nothing is connected, so there is nobody to ask.
    NoClient,
    /// The client went away between the send and the reply.
    Disconnected,
    /// No reply inside `COMMAND_TIMEOUT`.
    TimedOut,
    /// The client ran the command and it failed.
    Failed(String),
}

impl CommandBus {
    /// Send `command` and wait for its reply.
    /// `std::result::Result`, not this module's `Result`, which is the wire type a client sends
    /// back and which `run` unwraps into `CommandError::Failed`.
    pub async fn run(
        &self,
        command: Command,
    ) -> std::result::Result<serde_json::Value, CommandError>;

    /// Record the client that just spoke as the command target.
    pub fn set_target(&self, client: &Client);

    /// Hand a reply to whoever is waiting on it. A reply for an unknown id is logged and dropped:
    /// it is a reply to a command that already timed out.
    pub fn resolve(&self, reply: ReplyMessage);
}
```

`on_message` gains the bus:

```rust
-pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
+pub fn on_message(
+    client: &Client,
+    text: &str,
+    event_tx: &UnboundedSender<MercuryEvent>,
+    bus: &CommandBus,
+) {
     match serde_json::from_str::<Upstream>(text) {
         Ok(Upstream::Tab(TabMessage { url })) => {
             debug!(%url, "tab");
+            bus.set_target(client);
             let _ = event_tx.send(tab(url));
         }
-        Ok(Upstream::Reply(reply)) => {
-            debug!(id = reply.id, "reply with no command bus; dropping");
-        }
+        Ok(Upstream::Reply(reply)) => bus.resolve(reply),
         Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
     }
 }
```

The model never awaits. `state.handle` is synchronous and stays that way, so a command is an effect: `MercuryEffect::Browser(Command)` is performed by `perform_effect`, which spawns the `bus.run` and, when the reply lands, sends the outcome back in as an event. That is the same decoupling `Foreground` already has, where the effect asks and the watcher reports.

The event that carries a reply back into the model, and which handlers care about it, belong to the doc for whatever feature needs the answer first. Until one does, `CommandBus::run`'s result is logged.

Tests, over a fake client on a loopback port:

- `run` returns the `Result.Ok` payload for the id it sent.
- Two commands in flight resolve to their own replies, replies arriving out of order.
- A reply for an unknown id is dropped and leaves `pending` empty.
- No client is `NoClient` immediately, without waiting out the timeout.
- A client that never replies yields `TimedOut` and leaves `pending` empty.
