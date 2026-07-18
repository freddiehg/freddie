# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and later a way to drive Chrome directly (run page JavaScript, open and close tabs, read the selection). This extension is the bridge for both, over the loopback WebSocket mercury listens on.

This doc owns the browser side only. `external-events.md` owns mercury's side: the port, `MERCURY_PORT`, the `freddie_event_socket` crate, the token, the `Upstream` / `Downstream` / `Command` / `Result` types, and the mapping from a message to a `MercuryEvent`. Read it for the wire contract; this doc only builds the JSON it defines.

Two phases share one connection. v0 streams the URL up; the command bus adds commands down and results up.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it.

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep one warm: the worker's registered `chrome.tabs` listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. A dead socket during idle is correct, and it costs one reconnect on the next real event. A keepalive ping would be both a timer and pointless traffic.

## Change 1: v0, stream the active tab's URL

Two files under `chrome-extension/`. It depends on `external-events.md`'s Change 2 being live on `127.0.0.1:8797` and on `chrome-tab-url.md`'s `TabEvent`; with both, this ships the URL stream end to end.

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

`tabs` grants the `url` field on a `Tab`. `host_permissions` for the loopback lets the worker open the WebSocket: Chrome checks a `ws://` connection against the matching `http://` host permission, and match patterns ignore the port, so `http://127.0.0.1/*` covers `:8797`.

`chrome-extension/background.js`:

```js
// The mercury bridge service worker. Opens a loopback WebSocket to mercury and pushes the active
// tab's URL on every tab switch and in-tab navigation. Event-driven: it reconnects on the next
// event, with no keepalive timer and no poll.

const MERCURY_URL = "ws://127.0.0.1:8797";

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

## Change 2: the token

Depends on `external-events.md`'s Change 3, which makes mercury refuse a connection whose `?token=` does not match the one in its state directory.

`chrome-extension/options.html` and `chrome-extension/options.js` are a one-input page that reads and writes `chrome.storage.local.token`, and the manifest gains `"options_page": "options.html"` and the `storage` permission. mercury logs the token's path at startup; the user pastes the contents once, and it survives restarts of both sides.

`background.js` reads the token before connecting, so `connect` becomes async and `pushUrl` awaits it:

```js
async function mercuryUrl() {
  const { token } = await chrome.storage.local.get("token");
  return token ? `${MERCURY_URL}/?token=${encodeURIComponent(token)}` : MERCURY_URL;
}
```

A query parameter rather than a header or a handshake message: a service worker's `WebSocket` constructor cannot set headers, and putting it in the URL means an unauthorized client is refused at the handshake and never sends a frame.

## Change 3: the command bus

The bidirectional phase. mercury sends `Downstream.Command` down; the extension runs each with the real extension APIs and sends `Upstream.Reply` back up. It depends on `external-events.md`'s Change 4, which assigns the ids, holds the pending map, and enforces the timeout.

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

`command.value ?? {}` covers `Command.ReadSelection`, which is a unit variant and so arrives with no `value` at all.

`Command.RunJs` and `Command.ReadSelection` run in `world: "MAIN"` so page globals (`getSelection`, page functions) are in scope. `Command.CloseTabs` takes a Chrome URL match pattern in `url_glob`.

The listener is registered inside `connect`, on the socket it just created, so a reconnect after the worker was killed re-registers it.

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.
