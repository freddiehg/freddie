# Freddie Active Tab Reporter

Reports Chrome's active tab URL to a running mercury, which is how per-site key remaps know what
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

Pre-commit regenerates it on any Rust change and refuses the commit if the result differs from what
is checked in, so a stale copy cannot ship.
