# external effects: mercury driving the browser

Not built. `external-events.md` ships the socket and the direction where mercury is told things. This doc owns the other direction: mercury tells a connected client to do something.

Effects are fire and forget. `state.handle` returns `Vec<MercuryEffect>` and the effect loop performs each one; no effect returns a value to the model, and nothing waits. A command is one more effect, performed by writing it to the socket. `IncomingEvent` stays what `external-events.md` defines: a tab URL, and whatever later features add as events of their own.

## The vocabulary

One enum per level of the state tree that emits commands. `ChromeApp`'s level builds a `TabCommand`; `XSite`'s builds an `XCommand`. A handler cannot construct a command for a level it is not on.

```rust
// crates/mercury/src/external.rs

/// A message mercury sends down to a connected client.
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum OutgoingEffect {
    #[serde(rename = "OutgoingEffect.Command")]
    Command(Command),
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Command {
    #[serde(rename = "Command.Tab")]
    Tab(TabCommand),
    #[serde(rename = "Command.X")]
    X(XCommand),
}

/// The browser itself, from any page. `ChromeApp`'s level emits these.
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum TabCommand {
    #[serde(rename = "TabCommand.Open")]
    Open(OpenTab),
    #[serde(rename = "TabCommand.Close")]
    Close(CloseTabs),
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

/// x.com's list, emitted by `XSite` (`twitter-site.md`).
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum XCommand {
    #[serde(rename = "XCommand.SelectMove")]
    SelectMove(SelectMove),
    #[serde(rename = "XCommand.SelectAct")]
    SelectAct(SelectAct),
}

#[derive(serde::Serialize, Debug)]
pub struct SelectMove {
    pub delta: i32,
}

#[derive(serde::Serialize, Debug)]
pub struct SelectAct {
    pub action: XAction,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum XAction {
    #[serde(rename = "XAction.Open")]
    Open,
    #[serde(rename = "XAction.Like")]
    Like,
    #[serde(rename = "XAction.Reply")]
    Reply,
    #[serde(rename = "XAction.Repost")]
    Repost,
    #[serde(rename = "XAction.Bookmark")]
    Bookmark,
}
```

A level that splits splits its enum with it. When the extension reports which x.com page is front, `XSite` gains a derived child per page and `XCommand` gains `Timeline(TimelineCommand)` and `Post(PostCommand)`, each emitted by the level of the same name.

```jsonc
// mercury -> client
{ "kind": "OutgoingEffect.Command", "value": { "kind": "Command.X", "value": { "kind": "XCommand.SelectMove", "value": { "delta": 1 } } } }
{ "kind": "OutgoingEffect.Command", "value": { "kind": "Command.Tab", "value": { "kind": "TabCommand.Open", "value": { "url": "https://x.com/home" } } } }
```

Serialize only, so nothing here can arrive from outside.

## The effect

```rust
 pub enum MercuryEffect {
     Foreground(App),
     Tap(Chord),
     // …
+    /// Tell the connected browser to do something.
+    Browser(Command),
 }
```

x.com's `j`:

```rust
pub(crate) fn next_post<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Browser(Command::X(XCommand::SelectMove(
        SelectMove { delta: 1 },
    )))]
}
```

## Performing it

`freddie_event_socket` gains a handle to a connection:

```rust
/// A live client connection. Cloneable and cheap: it is a sender into that connection's write
/// task, so a caller can stash one and send to it later.
#[derive(Clone)]
pub struct Client {
    outgoing: UnboundedSender<String>,
}

impl Client {
    /// Queue `text` to this client.
    ///
    /// # Errors
    ///
    /// If the connection is gone.
    pub fn send(&self, text: String) -> Result<(), Disconnected>;
}

pub fn listen<F>(port: u16, auth: Auth, on_message: F) -> std::io::Result<EventSocket>
where
    F: Fn(&Client, &str) + Send + Sync + 'static;
```

mercury writes to the client that spoke most recently. The extension pushes a tab URL as soon as it connects, so that is the browser.

```rust
// crates/mercury/src/external.rs

/// Where commands go: the client that spoke most recently.
pub struct Browser {
    client: Mutex<Option<Client>>,
}

impl Browser {
    pub fn set(&self, client: &Client);

    /// Serialize `command` and write it. A failure is logged and dropped.
    pub fn send(&self, command: Command);
}
```

`on_message` records the sender, before:

```rust
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            debug!(%url, "tab");
            let _ = event_tx.send(tab(url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

after:

```rust
pub fn on_message(
    client: &Client,
    text: &str,
    event_tx: &UnboundedSender<MercuryEvent>,
    browser: &Browser,
) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            debug!(%url, "tab");
            browser.set(client);
            let _ = event_tx.send(tab(url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

`perform_effect` gains an arm:

```rust
+        MercuryEffect::Browser(command) => browser.send(command),
```

## The token

`external-events.md` refuses web pages at the handshake and asks for nothing else, because a client that can only report a tab URL can only lie about the front tab. A client that can close tabs and act on posts is worth gating.

mercury has to know the client is the extension: otherwise a local process connects, sends one tab frame to become the target, and receives what mercury meant for the browser. The extension has to know the server is mercury: otherwise a local process that binds 3883 first is a server it takes commands from.

One shared secret, checked both ways, pasted once.

```rust
// crates/mercury/src/external.rs

/// Read the persisted token, generating and writing one on first run.
///
/// Beside the single-instance lock, in the platform state directory: it survives across runs and is
/// not swept out of a temp directory. 16 bytes from `/dev/urandom`, hex encoded, `0o600`.
pub fn token() -> std::io::Result<String>;

/// Where [`token`] keeps it, for the log to name.
pub fn token_path() -> PathBuf;
```

`listen` gains the gate:

```rust
+/// What a connecting client presents, and what the server proves back.
+pub enum Auth {
+    Open,
+    /// The handshake is refused unless `?token=` matches, and its response carries
+    /// `sec-mercury-token` for the client to check.
+    Token(String),
+}
```

The client's half is a query parameter, which a Chrome service worker can attach, checked during the HTTP handshake and answered with 401 on a mismatch. The server's half rides the handshake response, so the extension drops the connection before acting on anything.

`chrome-extension/options.html` gains a second input, over `chrome.storage.local.token`. mercury logs `token_path` at startup.

## The extension's half

The manifest gains `scripting` and the hosts it acts on:

```json
{
  "permissions": ["tabs", "scripting", "storage"],
  "host_permissions": ["http://127.0.0.1/*", "https://x.com/*"]
}
```

A content script per site owns that site's list and its actions (`twitter-site.md`). The service worker holds the socket and forwards:

```ts
import type { OutgoingEffect } from "./wire/OutgoingEffect";

const outgoingEffect: z.ZodType<OutgoingEffect> = z.object({
  kind: z.literal("OutgoingEffect.Command"),
  value: command,
});

socket.addEventListener("message", (ev: MessageEvent<string>) => {
  const parsed = outgoingEffect.safeParse(JSON.parse(ev.data));
  if (!parsed.success) {
    console.error("mercury sent a frame this cannot read", parsed.error);
    return;
  }
  void deliver(parsed.data.value);
});

// `Command.Tab` is the worker's own; a site's command goes to that site's content script.
async function deliver(command: OutgoingEffect["value"]): Promise<void> {
  if (command.kind === "Command.Tab") {
    await runTabCommand(command.value);
    return;
  }
  const [tab] = await chrome.tabs.query({
    active: true,
    lastFocusedWindow: true,
  });
  if (tab?.id !== undefined) await chrome.tabs.sendMessage(tab.id, command);
}
```

`JSON.parse` returns `any`, so a frame off a socket any local process can reach is parsed rather than cast. Annotating the schema `z.ZodType<OutgoingEffect>` checks it against the ts-rs generated type, so a variant added in Rust that the schema does not follow fails `tsc`.

zod becomes the extension's first runtime dependency.

## Changes

1. The token: `token`, `token_path`, `Auth` on `listen`, both halves of the check, and the extension's second options input.
2. `Client` on the socket crate, `listen`'s callback growing a `&Client`, `on_message` taking it.
3. The vocabulary, `MercuryEffect::Browser`, `Browser`, and `perform_effect`'s arm. `TabCommand::Open` is the first bind that uses it.
4. The extension's dispatch: zod, the worker's forwarding, and x.com's content script.

Tests, over a fake client on a loopback port:

- Each command variant serializes to the nested form above.
- `Browser::send` with no client attached logs and returns.
- Of two clients that connect in turn, the one that spoke most recently is written to.
- A disconnected client's `send` is an error rather than a panic.
- A wrong `?token=` is refused at the handshake, and a handshake response without the token makes a client disconnect.
