# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and only the browser knows it. This extension is the bridge, over the loopback WebSocket mercury listens on.

This doc owns the browser side of one direction: the extension pushing the frontmost tab's URL up. `external-events.md` owns mercury's side of it, and is where the port, `MERCURY_PORT`, the `freddie_event_socket` crate, and the `IncomingEvent` vocabulary are defined; read it for the wire contract, because this doc only builds the JSON it defines.

Commands going the other way, the token they need, and the extension code that runs them are `external-effects.md`. They share this connection but nothing here anticipates them.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it.

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep one warm: the worker's registered `chrome.tabs` listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. A dead socket during idle is correct, and it costs one reconnect on the next real event. A keepalive ping would be both a timer and pointless traffic.

## The layout

`chrome-extension/` at the top level of the repository, beside `crates/`, not inside it. It is not a Rust crate and cargo has no business seeing it, and it is loaded unpacked from that path at `chrome://extensions`.

```
chrome-extension/
  README.md          how to install it and check that it works
  manifest.json
  background.js      the service worker: listeners, socket, and the frame it sends
  options.html       one number input, the port
  options.js
  types/wire.d.ts    generated from mercury's types; checked in
  jsconfig.json      turns on type checking for the JavaScript above
  eslint.config.js
  package.json       dev tooling only; the extension itself ships no dependencies
  .gitignore         node_modules
```

Nothing is compiled. Chrome loads these files as they are, so there is no build step between editing and reloading, and no output directory. The type checking below is a check that runs over the sources, not a compiler that produces different ones.

It depends on `external-events.md`'s Change 2 being live on `127.0.0.1:3883` and on `chrome-tab-url.md`'s `TabEvent`; with both, this ships the URL stream end to end.

## Stream the active tab's URL

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

`chrome-extension/background.js`, which `// @ts-check` puts under the type checker described below:

```js
// @ts-check
/** @typedef {import("./types/wire").IncomingEvent} IncomingEvent */

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
  /** @type {IncomingEvent} */
  const frame = { kind: "IncomingEvent.Tab", value: { url } };
  const payload = JSON.stringify(frame);
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

## Generating the wire types

The frame the extension builds is described by a type generated from the Rust that parses it, so a variant renamed in `external.rs` fails the extension's type check rather than failing silently at runtime.

`ts-rs` derives it. In `crates/mercury/Cargo.toml`:

```toml
[dependencies]
ts-rs = { version = "10", features = ["serde-compat"], optional = true }

[features]
# Off in every normal build: the derive exists to write a `.d.ts`, and nothing in mercury reads it.
typescript = ["dep:ts-rs"]
```

and on the types in `crates/mercury/src/external.rs`:

```rust
 #[derive(serde::Deserialize, Debug)]
+#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
+#[cfg_attr(feature = "typescript", ts(export, export_to = "../../chrome-extension/types/"))]
 #[serde(tag = "kind", content = "value")]
 pub enum IncomingEvent {
```

`serde-compat` is what makes it read the `serde` attributes, so the generated type carries the renames and the adjacent tagging rather than ts-rs's own defaults. Confirmed against ts-rs 10, which writes exactly the union form the project uses:

```ts
export type IncomingEvent = { "kind": "IncomingEvent.Tab", "value": TabMessage };
export type TabMessage = { url: string, };
```

ts-rs exports during a test run, so the command is:

```
cargo test -p mercury --features typescript export_bindings
```

The output is checked in, because the extension has no build step and a developer loading it unpacked should not need a Rust toolchain to have working types.

Checked in means it can go stale, so it is regenerated in both places that already run the tests, and both fail when what they produce is not what was committed. Without that the generated file is a copy that rots.

Neither place needs a new command to do the regenerating. `.pre-commit-config.yaml`'s `cargo-test` hook and CI's `cargo-test` job both run `cargo test --all --all-features`, and `--all-features` turns on `typescript`, so the export runs on every commit that touches Rust and on every push.

Pre-commit needs nothing at all. It compares the working tree before and after its hooks and refuses a commit whose hooks changed a file, which is the same failure `cargo fmt` produces when it reformats something: the commit stops, the regenerated file is sitting there, and it goes in with `git add`.

CI needs one step, in the `cargo-test` job, after the tests:

```yaml
      - name: Run cargo test
        run: cargo test --all --all-features
      - name: Wire types are up to date
        run: git diff --exit-code chrome-extension/types/
```

A pull request that changes `IncomingEvent` and forgets to commit the regenerated `.d.ts` fails there, naming the file.

## Type checking, linting, and formatting

`chrome-extension/package.json`:

```json
{
  "name": "mercury-bridge",
  "private": true,
  "scripts": {
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "format": "prettier --write .",
    "format:check": "prettier --check ."
  },
  "devDependencies": {
    "@types/chrome": "^0.0.279",
    "eslint": "^9",
    "prettier": "^3",
    "typescript": "^5"
  }
}
```

`typescript` is a dev dependency for `tsc --noEmit` alone. The extension stays JavaScript: TypeScript would mean compiling before every reload, which is the friction this layout exists to avoid, and `// @ts-check` with JSDoc gets the same checking over the files Chrome actually loads.

`chrome-extension/jsconfig.json`:

```json
{
  "compilerOptions": {
    "checkJs": true,
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "types": ["chrome"]
  },
  "include": ["*.js", "types/*.d.ts"]
}
```

`types: ["chrome"]` is what makes `chrome.tabs` and `chrome.storage` typed rather than `any`, so a misspelled listener or a wrong argument is caught here.

`chrome-extension/eslint.config.js`:

```js
import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    files: ["*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { chrome: "readonly", WebSocket: "readonly" },
    },
    rules: {
      "no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
];
```

Prettier runs with its defaults; there is no `.prettierrc`, because a config file that only restates the defaults is one more thing to disagree with.

These are not wired into `.pre-commit-config.yaml`. Its hooks are `types: [rust]` and run cargo, and adding node tooling there would put an `npm install` in the path of every Rust commit. Run them from `chrome-extension/`, and in CI as their own job, which is separate from the wire-type check above: that one belongs to the Rust job because a Rust change is what invalidates it.

## The README

`chrome-extension/README.md` is the only instructions there are, so it carries everything needed to go from a checkout to a working bridge:

````markdown
# mercury bridge

Pushes Chrome's active tab URL to a running mercury, which is how per-site key remaps know what
site you are on. It sends one message and takes none.

## Install

1. Open `chrome://extensions`.
2. Turn on Developer mode.
3. Load unpacked, and choose this directory.

Nothing is compiled and there is no build step: Chrome loads these files as they are. After editing
one, press the reload arrow on the extension's card.

## Configure

The port only needs setting if mercury is not on its default of 3883, which happens when it was
started with `--port` or `MERCURY_PORT`. From the extension's card, open Details, then Extension
options, and set it to match.

## Check that it works

With mercury running, from the repository root:

```
tail -f ~/Library/Logs/mercury/mercury.log
```

Switch tabs. Each switch should add a dispatch record whose state carries the new URL:

```
Foreground { app: Chrome(ForegroundedChrome { url: Some("https://claude.ai/new") }), ... }
```

Nothing at all means the frame never arrived. In order of likelihood: mercury is not running, the
port does not match, or the service worker threw. The worker's console is behind the "service
worker" link on the extension's card, and its errors show up there rather than in any page's
console.

## Develop

```
npm install
npm run typecheck
npm run lint
npm run format
```

`types/wire.d.ts` is generated from mercury's Rust types and checked in. Do not edit it. After
changing the wire format in `crates/mercury/src/external.rs`:

```
cargo test -p mercury --features typescript export_bindings
```
````

Install for development by loading `chrome-extension/` unpacked at `chrome://extensions`. A packed or store build is deferred; nothing in the code depends on which.

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.
