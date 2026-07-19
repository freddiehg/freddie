# the mercury Chrome extension

Not built. mercury needs Chrome's active-tab URL for per-site key remaps (`chrome-tab-url.md`), and only the browser knows it. This extension is the bridge, over the loopback WebSocket mercury listens on.

TypeScript, checked as strictly as the Rust is. The workspace denies clippy's `pedantic` and `nursery`, and the browser half is held to the same standard. The wire types are generated from mercury's own and imported, so a variant renamed in Rust fails `tsc` rather than failing at runtime.

This doc owns the browser side of one direction: the extension pushing the frontmost tab's URL up. `external-events.md` owns mercury's side of it, and is where the port, `MERCURY_PORT`, the `freddie_event_socket` crate, and the `IncomingEvent` vocabulary are defined.

Commands going the other way, and the token they need, are `external-effects.md`. They share this connection but nothing here anticipates them.

The extension pushes; it never answers a timer. There is no keepalive and no poll loop. When there is nothing to report the connection may lapse, and the next tab event reopens it.

## The service-worker lifetime

An MV3 background service worker is killed after roughly 30s idle, which closes the WebSocket. This is why there is no persistent connection to hold and no timer to keep one warm: the worker's registered listeners revive it when a tab event fires, and the handler reopens the socket if it is closed before sending. A dead socket during idle is correct, and it costs one reconnect on the next real event. A keepalive ping would be both a timer and pointless traffic.

## The layout

`chrome-extension/` at the top level of the repository, beside `crates/`, not inside it. It is not a Rust crate and cargo has no business seeing it.

```
chrome-extension/
  README.md            how to build it, install it, and check that it works
  manifest.json
  options.html
  package.json         pnpm, dev tooling only; the extension ships no dependencies
  pnpm-lock.yaml       checked in
  tsconfig.json
  eslint.config.js
  .gitignore           node_modules, dist
  src/
    background.ts      the service worker: listeners, socket, and the frame it sends
    options.ts         the port setting
    wire/              generated from mercury's types; checked in, never edited
      IncomingEvent.ts
      TabMessage.ts
  dist/                tsc output, gitignored, and what the manifest points at
```

Chrome loads `chrome-extension/`, and the manifest points into `dist/`, so `pnpm build` has to have run before the extension will load at all. That is what TypeScript costs here, and `pnpm watch` covers the edit loop.

## Change 1: the extension

It depends on `external-events.md`'s Change 2 being live on `127.0.0.1:3883` and on `chrome-tab-url.md`'s `TabEvent`; with both, this ships the URL stream end to end.

`chrome-extension/manifest.json`:

```json
{
  "manifest_version": 3,
  "name": "mercury bridge",
  "version": "0.0.1",
  "background": { "service_worker": "dist/background.js", "type": "module" },
  "permissions": ["tabs", "storage"],
  "host_permissions": ["http://127.0.0.1/*"],
  "options_page": "options.html"
}
```

`tabs` grants the `url` field on a `Tab`. `host_permissions` for the loopback lets the worker open the WebSocket: Chrome checks a `ws://` connection against the matching `http://` host permission, and match patterns carry no port, so `http://127.0.0.1/*` covers every port mercury might be on. `"type": "module"` matches the `ES2022` modules `tsc` emits.

`chrome-extension/src/background.ts`:

```ts
import type { IncomingEvent } from "./wire/IncomingEvent.js";

// mercury's default. The options page overrides it, and it has to match whatever `--port` or
// `MERCURY_PORT` mercury was given.
const DEFAULT_PORT = 3883;

let socket: WebSocket | null = null;

/** The configured port, or the default if the options page has never been used. */
async function port(): Promise<number> {
  const { port } = await chrome.storage.local.get({ port: DEFAULT_PORT });
  return typeof port === "number" ? port : DEFAULT_PORT;
}

/**
 * The socket to send on, opening one if there is none.
 *
 * Returns it rather than leaving the caller to read the module variable, which could by then hold a
 * different socket than the one it asked for.
 */
async function connect(): Promise<WebSocket> {
  if (
    socket !== null &&
    (socket.readyState === WebSocket.OPEN ||
      socket.readyState === WebSocket.CONNECTING)
  ) {
    return socket;
  }
  const ws = new WebSocket(`ws://127.0.0.1:${String(await port())}`);
  // Each handler clears only its own socket. A connection that fails fires `error` and then
  // `close`, and by then a later tab event may already have replaced `socket`; clearing
  // unconditionally would drop a live socket and strand whatever it was about to send.
  const forget = (): void => {
    if (socket === ws) socket = null;
  };
  ws.addEventListener("close", forget);
  ws.addEventListener("error", forget);
  socket = ws;
  return ws;
}

/**
 * Send `url` to mercury.
 *
 * A URL that cannot be sent is dropped rather than queued: the next tab event supersedes it, so a
 * retry would deliver a stale answer to a question nobody asked yet.
 */
async function pushUrl(url: string | undefined): Promise<void> {
  if (url === undefined || url === "") return;
  const ws = await connect();
  const frame: IncomingEvent = { kind: "IncomingEvent.Tab", value: { url } };
  const payload = JSON.stringify(frame);
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(payload);
  } else {
    ws.addEventListener("open", () => ws.send(payload), { once: true });
  }
}

chrome.tabs.onActivated.addListener(({ tabId }) => {
  void chrome.tabs.get(tabId).then((tab) => pushUrl(tab.url));
});

// `onUpdated` fires per changed tab, so this filters to the active one and to changes that
// actually carried a URL.
chrome.tabs.onUpdated.addListener((_tabId, info, tab) => {
  if (info.url !== undefined && tab.active) void pushUrl(info.url);
});

// Returning from another application, and switching between two Chrome windows, both change which
// tab is front with no tab event at all. `WINDOW_ID_NONE` means Chrome lost focus, so there is
// nothing to report.
chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) return;
  void chrome.tabs
    .query({ active: true, windowId })
    .then(([tab]) => pushUrl(tab?.url));
});
```

The wire type is a plain import. The `.js` extension on the specifier is what the emitted module needs at runtime, since Chrome resolves it and `tsc` rewrites nothing; `"moduleResolution": "bundler"` is what lets the source say `.js` while the file on disk is `.ts`.

Re-sending an identical URL is harmless: `record_tab_url` fills the same value and dispatch produces no change (`chrome-tab-url.md`).

`chrome-extension/options.html`:

```html
<!doctype html>
<meta charset="utf-8" />
<title>mercury bridge</title>
<label for="port">mercury's port</label>
<input id="port" type="number" min="1" max="65535" />
<p id="status"></p>
<script type="module" src="dist/options.js"></script>
```

`chrome-extension/src/options.ts`:

```ts
const DEFAULT_PORT = 3883;

const input = document.querySelector<HTMLInputElement>("#port");
const status = document.querySelector<HTMLParagraphElement>("#status");
if (input === null || status === null) {
  throw new Error("the options page is missing its input");
}

const { port } = await chrome.storage.local.get({ port: DEFAULT_PORT });
input.value = String(port);

input.addEventListener("change", () => {
  const chosen = Number(input.value);
  if (!Number.isInteger(chosen) || chosen < 1 || chosen > 65535) {
    status.textContent = "not a port number";
    return;
  }
  void chrome.storage.local.set({ port: chosen }).then(() => {
    status.textContent = `saved: mercury on ${String(chosen)}`;
  });
});
```

Storage is read on every push rather than cached, so a change here takes effect on the next tab event without reloading anything.

## Change 2: the generated wire types

The frame the extension builds is described by a type generated from the Rust that parses it, so a variant renamed in `external.rs` fails `tsc` rather than failing silently at runtime.

`ts-rs` derives it. In `crates/mercury/Cargo.toml`:

```toml
[dependencies]
ts-rs = { version = "10", features = ["serde-compat"], optional = true }

[features]
# Off in every normal build: the derive exists to write the `.ts` files, and nothing in mercury
# reads them.
typescript = ["dep:ts-rs"]
```

and on the types in `crates/mercury/src/external.rs`:

```rust
 #[derive(serde::Deserialize, Debug)]
+#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
+#[cfg_attr(feature = "typescript", ts(export, export_to = "../../chrome-extension/src/wire/"))]
 #[serde(tag = "kind", content = "value")]
 pub enum IncomingEvent {
```

`serde-compat` is what makes it read the `serde` attributes, so the output carries the renames and the adjacent tagging rather than ts-rs's own defaults. Confirmed against ts-rs 10, which writes exactly the union form the project uses:

```ts
export type IncomingEvent = { "kind": "IncomingEvent.Tab", "value": TabMessage };
export type TabMessage = { url: string, };
```

ts-rs exports during a test run:

```
cargo test -p mercury --features typescript export_bindings
```

The output is checked in, so loading the extension needs no Rust toolchain. Checked in means it can go stale, so it is regenerated in both places that already run the tests. Neither needs a new command: `.pre-commit-config.yaml`'s `cargo-test` hook and CI's `cargo-test` job both run `cargo test --all --all-features`, and `--all-features` turns on `typescript`.

Pre-commit needs nothing configured. It refuses a commit whose hooks changed a file, the same way `cargo fmt` does when it reformats something: the commit stops, the regenerated file is sitting there, and it goes in with `git add`.

CI needs one step in the `cargo-test` job, after the tests:

```yaml
      - name: Wire types are up to date
        run: git diff --exit-code chrome-extension/src/wire/
```

A pull request that changes `IncomingEvent` and forgets to commit the regenerated types fails there, naming the file.

## Change 3: the tooling

`chrome-extension/package.json`:

```json
{
  "name": "mercury-bridge",
  "private": true,
  "type": "module",
  "packageManager": "pnpm@9.12.0",
  "scripts": {
    "build": "tsc",
    "watch": "tsc --watch",
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "format": "prettier --write .",
    "format:check": "prettier --check ."
  },
  "devDependencies": {
    "@types/chrome": "^0.0.279",
    "eslint": "^9",
    "prettier": "^3",
    "typescript": "^5",
    "typescript-eslint": "^8"
  }
}
```

pnpm, pinned through `packageManager` so corepack uses one version everywhere, with `pnpm-lock.yaml` checked in. There are no runtime dependencies and there will not be: what ships to the browser is `tsc`'s output over the files above.

`chrome-extension/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "lib": ["ES2022", "DOM"],
    "types": ["chrome"],
    "rootDir": "src",
    "outDir": "dist",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true,
    "skipLibCheck": true
  },
  "include": ["src"]
}
```

Everything past `strict` is the point rather than decoration, and it is the same posture as clippy at `pedantic` and `nursery`. `noUncheckedIndexedAccess` is what makes `const [tab] = await chrome.tabs.query(...)` a `Tab | undefined`, which is why `pushUrl` takes `string | undefined` and checks it.

`chrome-extension/eslint.config.js`:

```js
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_" },
      ],
    },
  },
  { ignores: ["dist/", "src/wire/"] },
);
```

`strictTypeChecked` rather than `recommended`, for the reason clippy runs at `pedantic` and `nursery`. `src/wire/` is ignored because it is generated, and linting it would fight the generator.

Prettier runs with its defaults, so there is no `.prettierrc`: a config that only restates the defaults is one more thing to disagree with.

None of this is wired into `.pre-commit-config.yaml`. Those hooks are `types: [rust]` and run cargo, and putting node tooling there would mean a `pnpm install` in the path of every Rust commit. CI runs them as their own job:

```yaml
  extension:
    name: extension
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: chrome-extension
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: chrome-extension/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm run typecheck
      - run: pnpm run lint
      - run: pnpm run format:check
```

and `extension` joins `all-checks-passed`'s `needs` list, so a broken extension blocks a release the way a broken crate does.

## Change 4: the README

`chrome-extension/README.md`:

````markdown
# mercury bridge

Pushes Chrome's active tab URL to a running mercury, which is how per-site key remaps know what
site you are on. It sends one message and takes none.

## Build

```
pnpm install
pnpm build
```

Chrome loads what `tsc` writes into `dist/`, so this has to run before the extension will load, and
again after every edit. While working on it, `pnpm watch` rebuilds on save.

## Install

1. Build, above.
2. Open `chrome://extensions`.
3. Turn on Developer mode.
4. Load unpacked, and choose this directory.

Rebuilding does not reload the extension. Press the reload arrow on its card after every build.

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
port does not match, or the service worker threw. Its console is behind the "service worker" link
on the extension's card, and its errors appear there rather than in any page's console.

## Develop

```
pnpm typecheck
pnpm lint
pnpm format
```

`src/wire/` is generated from mercury's Rust types and checked in. Do not edit it. After changing
the wire format in `crates/mercury/src/external.rs`:

```
cargo test -p mercury --features typescript export_bindings
```
````

## Other browsers

One extension serves the Chromium browsers that support MV3 with the same `chrome.*` APIs (Chrome, Brave, Arc). They differ only in bundle id, which is mercury's concern through `App::from_bundle_id`, not the extension's. Safari has no equivalent loopback-WebSocket path and is out of scope.

## To confirm while building this

- That `chrome.windows.onFocusChanged` fires when focus returns from another application, rather than only between Chrome windows, and whether devtools taking focus produces a spurious push.
- Whether `onUpdated` carries `info.url` for same-document navigations. It is the event for document loads; `chrome.webNavigation.onHistoryStateUpdated` and `onReferenceFragmentUpdated` exist for History API and fragment changes. This costs nothing while `Site::from_url` matches on the host, since an SPA route change inside `claude.ai` leaves the host alone, and it starts mattering the moment a bind keys on the path.
- That ts-rs writes `IncomingEvent.ts` importing `TabMessage` with a specifier the `.js`-extension rule above accepts. If it emits an extensionless one, `"moduleResolution": "bundler"` resolves it for `tsc` and it is erased at emit anyway, since both are type-only.
