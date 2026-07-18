# external effects: mercury driving the browser

Not built, and downstream of `external-events.md`, which ships the socket and owns the direction where mercury is told things. This doc owns the other direction: mercury asks a connected client to do something and uses the answer.

"Run this JavaScript in the front tab" is an effect, the way `Foreground` and `Tap` are effects. The model returns it, the effect loop performs it by writing it to the socket, and what comes back up is a reply. Both halves are here: mercury's, and the extension's handler table (`chrome-extension.md` ships the extension that this extends).

The motivating actions are the ones no keystroke expresses: open the front tab's URL in Zed, read the selection, close every tab matching a pattern, pull a value out of the page.

## `Downstream` is serialize-only

Nothing in this direction has a `Deserialize` impl, so a command cannot arrive from outside even by accident. That is the same reasoning `external-events.md` applies to `Upstream`, pointed the other way.

```rust
// crates/mercury/src/external.rs

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

/// What a `MercuryEffect::Browser` carries. An effect, never an event.
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

/// Monotonic per-process, so a reply is never mistaken for a reply to an earlier command.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct CommandId(u64);
```

The reply is the one thing in this doc that comes up the socket, so it is a variant of `external-events.md`'s `Upstream`:

```rust
 pub enum Upstream {
     #[serde(rename = "Upstream.Tab")]
     Tab(TabMessage),
+    /// A reply to a command mercury sent down. The command bus consumes it; it is never dispatched
+    /// as an event, because the effect that sent the command is what is waiting for it.
+    #[serde(rename = "Upstream.Reply")]
+    Reply(ReplyMessage),
 }

+#[derive(serde::Deserialize, Debug)]
+pub struct ReplyMessage {
+    pub id: CommandId,
+    pub result: Result,
+}

+/// What a command produced. Deserialize only: it arrives inside an `Upstream::Reply`.
+///
+/// It shadows the prelude's `Result` in this module, which is the point: it is a result, with the
+/// same `Ok(value)` / `Err(message)` meaning, and the union convention makes the wire tag the type
+/// name, so shadowing is what gets `Result.Ok` / `Result.Err` on the wire. Anything in this module
+/// that wants the prelude's spells it `std::result::Result`.
+#[derive(serde::Deserialize, Debug)]
+#[serde(tag = "kind", content = "value")]
+pub enum Result {
+    /// Whatever JSON the handler returned.
+    #[serde(rename = "Result.Ok")]
+    Ok(serde_json::Value),
+    /// The message from a command that threw.
+    #[serde(rename = "Result.Err")]
+    Err(String),
+}
```

```jsonc
// mercury -> client
{ "kind": "Downstream.Command", "value": { "id": 42, "command": { "kind": "Command.RunJs", "value": { "code": "document.title" } } } }
{ "kind": "Downstream.Command", "value": { "id": 43, "command": { "kind": "Command.ReadSelection" } } }

// client -> mercury
{ "kind": "Upstream.Reply", "value": { "id": 42, "result": { "kind": "Result.Ok", "value": "hello" } } }
{ "kind": "Upstream.Reply", "value": { "id": 43, "result": { "kind": "Result.Err", "value": "no active tab" } } }
```

A unit variant like `Command.ReadSelection` carries no `value` at all.

## The token

`external-events.md` ships no auth, and says why: a client that can only report a tab URL is a client whose worst move is a lie about the current tab. This doc breaks that. A `Command.RunJs` runs arbitrary JavaScript in a logged-in browser session, so both ends have to know who they are talking to, and both directions of that are worth stating plainly.

- mercury has to know the client is the extension. Otherwise a local process connects, sends one tab message to become the command target, and receives the commands mercury meant for the browser.
- The extension has to know the server is mercury. This is the sharper one. A local process that binds 8797 before mercury starts is a server the extension will connect to and take commands from, and those commands run in your tabs. A client-presents-a-secret scheme does nothing about it, because the impostor can accept any secret it is handed.

One shared secret, checked in both directions, covers both. The user pastes it once.

```rust
// crates/mercury/src/external.rs

/// Read the persisted token, generating and writing one on first run.
///
/// Lives beside the single-instance lock, in the platform state directory, for the same reason: it
/// must survive across runs and must not be swept out of a temp directory. 16 bytes from
/// `/dev/urandom`, hex encoded, and the file is created `0o600`.
///
/// Persisted rather than minted per run, because a per-run token means re-pasting into the
/// extension's options page after every restart, which is the kind of friction that ends with the
/// gate turned off.
pub fn token() -> std::io::Result<String>;

/// Where `token` keeps it, so the log can name the file for the user to open.
pub fn token_path() -> PathBuf;
```

`freddie_event_socket::listen` grows the gate:

```rust
 pub fn listen<F>(port: u16, auth: Auth, on_message: F) -> std::io::Result<EventSocket>
 where
     F: Fn(&Client, &str) + Send + Sync + 'static;

+/// What a connecting client must present, and what the server proves back.
+pub enum Auth {
+    /// Anyone on loopback may connect.
+    Open,
+    /// The connection is refused unless its `?token=` matches, and the handshake response carries
+    /// `sec-mercury-token` so the client can tell a real server from a squatter.
+    Token(String),
+}
```

The client's half of the token is checked during the HTTP handshake against the request URI's `token` query parameter; a mismatch is answered with 401 and the connection dropped before any frame is read. A query parameter is what a Chrome service worker can attach without a CSP fight, and checking at the handshake means an unauthorized client never gets to send anything.

The server's half rides the handshake response, so the extension can drop the connection before running anything if the header is missing or wrong.

## The model never awaits

`state.handle` is synchronous and stays that way. A command is an effect:

```rust
 pub enum MercuryEffect {
     Foreground(App),
     Tap { key: Key, flags: ModifierFlags },
     // …
+    /// Ask the connected browser to do something. Fire-and-forget from the effect loop's side; the
+    /// answer comes back as an event, the way `Foreground` is answered by the app-nav watcher.
+    Browser(Command),
 }
```

`perform_effect` spawns the send and the wait, and feeds the outcome back in as an event, which is the same decoupling `Foreground` already has: the effect asks, and something else reports what happened.

```rust
// crates/mercury/src/external.rs

/// A command that got no reply in this long is dropped and its waiter told `TimedOut`. A page that
/// hangs must not leak an entry per command forever.
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
    ///
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

`on_message` takes the client and the bus, which is why `listen`'s callback grows a `&Client`:

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
+        Ok(Upstream::Reply(reply)) => bus.resolve(reply),
         Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
     }
 }
```

`freddie_event_socket` gains the type that makes talking back possible:

```rust
/// A live client connection. Cloneable and cheap: it is a sender into that connection's write task,
/// so a caller can stash one and send to it later.
#[derive(Clone)]
pub struct Client {
    outgoing: UnboundedSender<String>,
}

impl Client {
    /// Queue `text` to this client. `Err` means the connection is gone.
    pub fn send(&self, text: String) -> std::result::Result<(), Disconnected>;
}
```

Its `max_message_size` goes from 64 KiB to 1 MiB: a `Command.RunJs` result is whatever the page returned, which is bigger than a URL.

## The extension's half

The manifest gains `scripting` (to inject into pages) and host permissions for the sites it will script:

```json
{
  "permissions": ["tabs", "scripting", "storage"],
  "host_permissions": ["http://127.0.0.1/*", "<all_urls>"]
}
```

A handler table maps each command variant to the API call that runs it, and a `message` listener dispatches by variant and replies with the result or the caught error:

```js
async function activeTab() {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  return tab;
}

const HANDLERS = {
  "Command.RunJs": async ({ code }) => {
    const tab = await activeTab();
    const [r] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      world: "MAIN",
      args: [code],
      func: (src) => eval(src),
    });
    return r.result;
  },
  "Command.OpenTab": async ({ url }) => {
    await chrome.tabs.create({ url });
    return null;
  },
  "Command.CloseTabs": async ({ url_glob }) => {
    const tabs = await chrome.tabs.query({ url: url_glob });
    await chrome.tabs.remove(tabs.map((t) => t.id));
    return null;
  },
  "Command.ReadSelection": async () => {
    const tab = await activeTab();
    const [r] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      world: "MAIN",
      func: () => String(getSelection()),
    });
    return r.result;
  },
};

socket.addEventListener("message", async (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.kind !== "Downstream.Command") return;
  const { id, command } = msg.value;
  let result;
  try {
    const value = await HANDLERS[command.kind](command.value ?? {});
    result = { kind: "Result.Ok", value };
  } catch (e) {
    result = { kind: "Result.Err", value: String(e) };
  }
  socket.send(JSON.stringify({ kind: "Upstream.Reply", value: { id, result } }));
});
```

`command.value ?? {}` covers `Command.ReadSelection`, which is a unit variant and so arrives with no `value`.

`Command.RunJs` and `Command.ReadSelection` run in `world: "MAIN"` so page globals (`getSelection`, page functions) are in scope. `Command.CloseTabs` takes a Chrome URL match pattern in `url_glob`.

The listener is registered inside `connect`, on the socket it just created, so a reconnect after the service worker was killed re-registers it.

## Changes

1. The token: `token`, `token_path`, `Auth` on `listen`, both directions of the check, and the extension's options page (`chrome-extension/options.html` and `options.js`, a one-input page over `chrome.storage.local.token`, with `"options_page"` and the `storage` permission in the manifest). mercury logs `token_path` at startup so the user can open it. It ships before anything can be commanded.
2. `Client` and `Downstream` on the socket crate, plus the raised message size. `listen`'s callback grows its `&Client`, and `on_message` takes it and ignores it.
3. The bus: `CommandBus`, `MercuryEffect::Browser`, `perform_effect` spawning it, and `Upstream::Reply` resolving it. The outcome is logged, since no handler wants it yet.
4. The extension's handler table.

The event that carries an answer back into the model, and which handler wants it, belong to the doc for whatever feature needs one first.

Tests for the bus, over a fake client on a loopback port:

- `run` returns the `Result.Ok` payload for the id it sent.
- Two commands in flight resolve to their own replies, with replies arriving out of order.
- A reply for an unknown id is dropped and leaves `pending` empty.
- No client is `NoClient` immediately, without waiting out the timeout.
- A client that never replies yields `TimedOut` and leaves `pending` empty.
- A wrong `?token=` is refused at the handshake, and a handshake response without the token makes a client disconnect.
