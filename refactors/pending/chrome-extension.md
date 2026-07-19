# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and only the browser knows it. This extension is the bridge, over the loopback WebSocket mercury listens on.

This doc owns the browser side of one direction: the extension pushing the frontmost tab's URL up. `external-events.md` owns mercury's side of it, and is where the port, `MERCURY_PORT`, the `freddie_event_socket` crate, and the `IncomingEvent` vocabulary are defined; read it for the wire contract, because this doc only builds the JSON it defines.

Commands going the other way, the token they need, and the extension code that runs them are `external-effects.md`. They share this connection but nothing here anticipates them.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it.

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep one warm: the worker's registered `chrome.tabs` listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. A dead socket during idle is correct, and it costs one reconnect on the next real event. A keepalive ping would be both a timer and pointless traffic.

## Stream the active tab's URL

`chrome-extension/` at the top level of the repository, beside `crates/`, not inside it. It is not a Rust crate and cargo has no business seeing it, and it is loaded unpacked from that path at `chrome://extensions`.

Four files. It depends on `external-events.md`'s Change 2 being live on `127.0.0.1:3883` and on `chrome-tab-url.md`'s `TabEvent`; with both, this ships the URL stream end to end.

`chrome-extension/manifest.json`:

```json
{
  "manifest_version": 3,
  "name": "mercury bridge",
  "version": "0.0.1",
  "background": { "service_worker": "background.js" },
  "permissions": ["tabs", "storage"],
  "host_permissions": ["http://127.0.0.1/*"],
  "options_page": "options.html"
}
```

`tabs` grants the `url` field on a `Tab`. `host_permissions` for the loopback lets the worker open the WebSocket: Chrome checks a `ws://` connection against the matching `http://` host permission, and match patterns ignore the port, so `http://127.0.0.1/*` covers every port mercury might be on.

`storage` and the options page are the port. mercury's port moves with `--port` or `MERCURY_PORT` (`external-events.md`), and an extension pinned to one number would connect to nothing the moment it does, with no symptom beyond per-site binds quietly not working. `DEFAULT_PORT` is the default on both sides, so the setting stays untouched unless you move mercury.

`chrome-extension/background.js`:

```js
// The mercury bridge service worker. Opens a loopback WebSocket to mercury and pushes the active
// tab's URL on every tab switch and in-tab navigation. Event-driven: it reconnects on the next
// event, with no keepalive timer and no poll.

// mercury's default. Overridden from the options page, which has to match whatever `--port` or
// `MERCURY_PORT` mercury was given.
const DEFAULT_PORT = 3883;

let socket = null;

async function mercuryUrl() {
  const { port } = await chrome.storage.local.get({ port: DEFAULT_PORT });
  return `ws://127.0.0.1:${port}`;
}

// Returns the socket to send on, so nothing downstream re-reads the module variable and finds a
// different socket than the one it asked for.
async function connect() {
  if (
    socket &&
    (socket.readyState === WebSocket.OPEN ||
      socket.readyState === WebSocket.CONNECTING)
  ) {
    return socket;
  }
  const ws = new WebSocket(await mercuryUrl());
  // Each handler clears only its own socket. A connection that fails fires `error` then `close`,
  // and by the time `close` runs a later tab event may already have replaced `socket`; an
  // unconditional `socket = null` would clobber the live one and strand its pending send.
  ws.addEventListener("close", () => {
    if (socket === ws) socket = null;
  });
  ws.addEventListener("error", () => {
    if (socket === ws) socket = null;
  });
  socket = ws;
  return ws;
}

async function pushUrl(url) {
  if (!url) return;
  const ws = await connect();
  const payload = JSON.stringify({ kind: "IncomingEvent.Tab", value: { url } });
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(payload);
  } else {
    ws.addEventListener("open", () => ws.send(payload), { once: true });
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

`onActivated` reads the tab that was just activated by id; `onUpdated` fires per changed tab, so it filters to `tab.active` and to updates that actually carried a new `url`. Re-sending an identical URL is harmless: `record_tab_url` fills the same value and dispatch produces no change (`chrome-tab-url.md`).

The frame shape is `{ kind: "IncomingEvent.Tab", value: { url } }`, which is `external-events.md`'s `IncomingEvent::Tab`. Confirmed against a running mercury: that exact frame produces a dispatch, and anything else is logged and dropped.

`onUpdated` covers document loads. It is not the event for a same-document navigation, which is what an SPA does when it changes route through the History API or a fragment; `chrome.webNavigation.onHistoryStateUpdated` and `onReferenceFragmentUpdated` exist for those, and whether `onUpdated` happens to carry `info.url` for them varies by version. This costs nothing today, because `Site::from_url` matches on the host and a route change inside `claude.ai` leaves the host alone. It starts mattering the moment a bind keys on the path, and that is the change that has to add `webNavigation` rather than assume `onUpdated` grew to cover it.

`chrome-extension/options.html` and `chrome-extension/options.js` are one number input over `chrome.storage.local.port`, defaulting to `DEFAULT_PORT`. Changing it takes effect on the next connection, which is the next tab event, because the worker reads storage every time it opens a socket rather than caching the value.

Install for development by loading `chrome-extension/` unpacked at `chrome://extensions`. A packed or store build is deferred; nothing in the code depends on which.

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.
