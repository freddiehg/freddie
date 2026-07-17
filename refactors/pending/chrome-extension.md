# a Chrome extension bridge for mercury

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and later a channel to drive Chrome directly (run page JavaScript, open and close tabs, read the selection). A Chrome extension is the bridge for both, over the localhost WebSocket that `external-events.md` defines. Two phases share one connection: v0 streams the URL up; the command bus adds commands down and results up.

This doc owns the browser side (the extension) and the wire contract both sides speak. The mercury side is split off: `external-events.md` owns the WebSocket source that terminates the connection and maps each message to a `MercuryEvent`, and `chrome-tab-url.md` owns the `TabEvent { url }` and the model that remaps keys from it.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it (see "The service-worker lifetime" below).

## The wire contract

One tagged JSON envelope, chosen up front so v0 and the command bus share it and shipping the bus does not rev the v0 message format. Every message is a JS discriminated union in the project's one form, `{ kind: "Type.Variant", value: T }`: `kind` is the dotted union-and-variant name and `value` is the whole payload.

```jsonc
// extension -> mercury
{ "kind": "Upstream.Tab", "value": { "url": "https://claude.ai/new" } }
{ "kind": "Upstream.Reply", "value": { "id": 42, "result": { "kind": "Result.Ok", "value": "…" } } }
{ "kind": "Upstream.Reply", "value": { "id": 42, "result": { "kind": "Result.Err", "value": "no active tab" } } }

// mercury -> extension
{ "kind": "Downstream.Command", "value": { "id": 42, "command": { "kind": "Command.RunJs", "value": { "code": "document.title" } } } }
```

The Rust side of the contract, in the WebSocket source (`external-events.md` wires these into the event loop; they are written here because they are the shared contract):

```rust
/// A message the extension sends up to mercury.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Upstream {
    /// The active tab changed URL. The v0 stream; benign, needs no token.
    #[serde(rename = "Upstream.Tab")]
    Tab { url: String },
    /// A reply to a command mercury sent down. Only after the command bus ships.
    #[serde(rename = "Upstream.Reply")]
    Reply { id: u64, result: Result },
}

/// A message mercury sends down to the extension. Only the command bus uses it.
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Downstream {
    #[serde(rename = "Downstream.Command")]
    Command { id: u64, command: Command },
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Command {
    #[serde(rename = "Command.RunJs")]
    RunJs { code: String },
    #[serde(rename = "Command.OpenTab")]
    OpenTab { url: String },
    #[serde(rename = "Command.CloseTabs")]
    CloseTabs { url_glob: String },
    #[serde(rename = "Command.ReadSelection")]
    ReadSelection,
}

/// Named `Result` (shadowing the prelude in this module) so its wire tag is
/// `Result.Ok`/`Result.Err` and its `Type` prefix matches the type, per the union
/// convention. Refer to the prelude's as `std::result::Result` here.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Result {
    /// The command's return value, whatever JSON the handler produced.
    #[serde(rename = "Result.Ok")]
    Ok(serde_json::Value),
    /// The error message from a command that threw.
    #[serde(rename = "Result.Err")]
    Err(String),
}
```

`Upstream::Tab` maps to `TabEvent { url }` (`chrome-tab-url.md`); the source drops any other upstream variant until the bus exists.

### The endpoint

mercury listens on `127.0.0.1:48291`. The port is the shared constant; `external-events.md`'s source binds it and the extension connects to it. Loopback-only is the security floor for v0 (see "Auth").

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep it warm: the worker's registered `chrome.tabs` listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. So a dead socket during idle is correct, not a bug, and it costs one reconnect on the next real event. No keepalive ping, which would be both a timer and pointless traffic.

## Change 1: v0, stream the active tab's URL

The whole v0 extension is two files under `chrome-extension/`. It depends on `external-events.md`'s WebSocket source being live on `127.0.0.1:48291` and on `chrome-tab-url.md`'s `TabEvent`; with both, this ships the URL stream end to end.

`chrome-extension/manifest.json`:

```json
{
  "manifest_version": 3,
  "name": "mercury bridge",
  "version": "0.0.1",
  "background": { "service_worker": "background.js" },
  "permissions": ["tabs"],
  "host_permissions": ["http://127.0.0.1/*"]
}
```

`tabs` grants the `url` field on a `Tab`. `host_permissions` for the loopback lets the worker open the WebSocket; Chrome checks a `ws://` connection against the matching `http://` host permission, and match patterns ignore the port, so `http://127.0.0.1/*` covers `:48291`.

`chrome-extension/background.js`:

```js
// The mercury bridge service worker. Opens a loopback WebSocket to mercury and
// pushes the active tab's URL on every tab switch and in-tab navigation.
// Event-driven: reconnect on the next event, no keepalive timer, no poll.

const MERCURY_URL = "ws://127.0.0.1:48291";

let socket = null;

function connect() {
  if (
    socket &&
    (socket.readyState === WebSocket.OPEN ||
      socket.readyState === WebSocket.CONNECTING)
  ) {
    return;
  }
  socket = new WebSocket(MERCURY_URL);
  socket.addEventListener("close", () => (socket = null));
  socket.addEventListener("error", () => (socket = null));
}

function pushUrl(url) {
  if (!url) return;
  connect();
  const payload = JSON.stringify({ kind: "Upstream.Tab", value: { url } });
  if (socket.readyState === WebSocket.OPEN) {
    socket.send(payload);
  } else {
    socket.addEventListener("open", () => socket.send(payload), { once: true });
  }
}

connect();

chrome.tabs.onActivated.addListener(({ tabId }) => {
  chrome.tabs.get(tabId, (tab) => pushUrl(tab.url));
});

chrome.tabs.onUpdated.addListener((_tabId, info, tab) => {
  if (info.url && tab.active) pushUrl(tab.url);
});
```

`onActivated` reads the tab that was just activated by id; `onUpdated` fires per changed tab, so it filters to `tab.active` and to updates that actually carried a new `url`. Re-sending an identical URL is harmless: `on_tab` fills the same value and dispatch produces no change (`chrome-tab-url.md`).

Install for development by loading `chrome-extension/` unpacked at `chrome://extensions`. A packed or store build is deferred; nothing in the code depends on which.

## Change 2: the command bus

The bidirectional phase. mercury sends `Downstream::Command` down; the extension runs each with the real extension APIs and sends `Upstream::Reply` back up. It depends on `external-events.md`'s source going bidirectional: assigning a monotonic `id` per command, holding an `id -> oneshot::Sender<Result>` map with a per-command timeout, and enforcing the token gate below.

### The extension's half

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

`Command.RunJs` and `Command.ReadSelection` run in `world: "MAIN"` so page globals (`getSelection`, page functions) are in scope; `Command.CloseTabs` takes a Chrome URL match pattern in `url_glob`.

### Auth

v0 ships tokenless: loopback plus a vocabulary restricted to `Upstream::Tab` is the boundary, and a tab URL from a stray local process is benign. The bus can run arbitrary JavaScript in your tabs, so it gates on a shared token.

mercury generates a token at startup and logs it. The user pastes it into the extension's options page, which stores it in `chrome.storage.local`; the extension appends it to the connect URL, `ws://127.0.0.1:48291/?token=…`, and mercury's source rejects the connection if it does not match. A query-param token is CSP-friendly from a service worker and needs no extra handshake message.

`chrome-extension/options.html` and `chrome-extension/options.js` are a one-input page that reads and writes `chrome.storage.local.token`; the manifest adds `"options_page": "options.html"`. `background.js` reads the token before connecting:

```js
async function mercuryUrl() {
  const { token } = await chrome.storage.local.get("token");
  return token ? `${MERCURY_URL}/?token=${encodeURIComponent(token)}` : MERCURY_URL;
}
```

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.
