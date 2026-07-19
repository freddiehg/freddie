# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and only the browser knows it. This extension is the bridge, over the loopback WebSocket mercury listens on.

This doc owns the browser side of one direction: the extension pushing the frontmost tab's URL up. `external-events.md` owns mercury's side of it, and is where the port, `MERCURY_PORT`, the `freddie_event_socket` crate, and the `IncomingEvent` vocabulary are defined; read it for the wire contract, because this doc only builds the JSON it defines.

Commands going the other way, the token they need, and the extension code that runs them are `external-effects.md`. They share this connection but nothing here anticipates them.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it.

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep one warm: the worker's registered `chrome.tabs` listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. A dead socket during idle is correct, and it costs one reconnect on the next real event. A keepalive ping would be both a timer and pointless traffic.

## Stream the active tab's URL

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
  const payload = JSON.stringify({ kind: "IncomingEvent.Tab", value: { url } });
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

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.
