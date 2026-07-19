# the extension performs a command

`external-effects.md` writes a command frame down the socket and `scoped-commands.md` gives it its shape. This is the browser half: the extension reads a frame, routes it to the site it names, and claude.ai's module puts settings up in the tab the command was addressed to.

It lands after both, and takes its types from `src/wire/` as they stand once `scoped-commands.md` has shipped:

```ts
export type OutgoingEffect = { kind: "OutgoingEffect.Command"; value: BrowserCommand };
export type BrowserCommand = { tab: TabId; command: SiteCommand };
export type TabId = number;
export type SiteCommand = { kind: "SiteCommand.ClaudeAi"; value: ClaudeAiCommand };
export type ClaudeAiCommand = { kind: "ClaudeAiCommand.OpenSettings"; value: SettingsSection };
export type SettingsSection = { kind: "SettingsSection.General"; value: null };
```

## The build bundles

A frame off a socket that any local process can reach is parsed, not cast: `JSON.parse` returns `any`, and the worker acts on what comes back. zod does the parsing, and a runtime dependency needs a bundler, because `tsc` emits the import as it was written and a service worker cannot resolve a bare specifier at load time.

`package.json`, before:

```json
  "scripts": {
    "build": "pnpm run icons && tsc",
    "watch": "pnpm run icons && tsc --watch",
    "typecheck": "tsc --noEmit",
```

after:

```json
  "scripts": {
    "build": "pnpm run icons && pnpm run typecheck && pnpm run bundle",
    "watch": "pnpm run icons && pnpm run bundle -- --watch",
    "bundle": "esbuild src/background.ts src/options.ts --bundle --format=esm --target=chrome120 --outdir=dist",
    "typecheck": "tsc --noEmit",
```

with `zod` as the first runtime dependency and `esbuild` beside the other tools:

```json
  "dependencies": {
    "zod": "^4"
  },
  "devDependencies": {
    "esbuild": "^0.25",
    …
  }
```

`tsconfig.json` stops emitting, since esbuild writes `dist/` now:

```json
    "noEmit": true,
```

esbuild does no type checking, so `build` runs `typecheck` first and a type error still fails the build. `watch` does not, which is what the editor is for.

## Frames are parsed

`src/frames.ts`, new. The schemas are annotated with the generated types, so a variant that changes in Rust fails `tsc` here rather than silently failing to match at runtime:

```ts
import { z } from "zod";

import type { BrowserCommand } from "./wire/BrowserCommand";
import type { ClaudeAiCommand } from "./wire/ClaudeAiCommand";
import type { OutgoingEffect } from "./wire/OutgoingEffect";
import type { SettingsSection } from "./wire/SettingsSection";
import type { SiteCommand } from "./wire/SiteCommand";

const settingsSection: z.ZodType<SettingsSection> = z.object({
  kind: z.literal("SettingsSection.General"),
  value: z.null(),
});

const claudeAiCommand: z.ZodType<ClaudeAiCommand> = z.object({
  kind: z.literal("ClaudeAiCommand.OpenSettings"),
  value: settingsSection,
});

const siteCommand: z.ZodType<SiteCommand> = z.object({
  kind: z.literal("SiteCommand.ClaudeAi"),
  value: claudeAiCommand,
});

const browserCommand: z.ZodType<BrowserCommand> = z.object({
  tab: z.number(),
  command: siteCommand,
});

const outgoingEffect: z.ZodType<OutgoingEffect> = z.object({
  kind: z.literal("OutgoingEffect.Command"),
  value: browserCommand,
});

/** The frame mercury sent, or `null` if it is not one this build understands. */
export function parseFrame(text: string): OutgoingEffect | null {
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (e) {
    console.error("mercury sent something that is not JSON", e);
    return null;
  }
  const parsed = outgoingEffect.safeParse(json);
  if (!parsed.success) {
    console.error("mercury sent a frame this build cannot read", parsed.error);
    return null;
  }
  return parsed.data;
}
```

## claude.ai's module

`src/sites/claudeAi.ts`, new. It takes a `ClaudeAiCommand` and cannot be handed another site's:

```ts
import type { ClaudeAiCommand } from "../wire/ClaudeAiCommand";
import type { SettingsSection } from "../wire/SettingsSection";
import type { TabId } from "../wire/TabId";

const HOST = "claude.ai";

/** The site's route for each section, which is the one place a URL is spelled. */
const SECTION_ROUTE: Record<SettingsSection["kind"], string> = {
  "SettingsSection.General": "settings/general",
};

export async function runClaudeAiCommand(
  tab: TabId,
  command: ClaudeAiCommand,
): Promise<void> {
  switch (command.kind) {
    case "ClaudeAiCommand.OpenSettings":
      await openSettings(tab, command.value);
      return;
  }
}

/**
 * Put a settings page up by putting its route in the tab's fragment.
 *
 * The URL differs from the current one only in its fragment, so Chrome navigates within the
 * document and the site's router opens settings; nothing reloads and no script is injected into
 * the page.
 */
async function openSettings(tab: TabId, section: SettingsSection): Promise<void> {
  const current = await chrome.tabs.get(tab);
  if (current.url === undefined) return;
  const url = new URL(current.url);
  // The command was computed against what this tab held when it reported. A tab navigates on its
  // own, so a command for claude.ai must not act on whatever is there now.
  if (url.host !== HOST) {
    console.warn(`a claude.ai command reached ${url.host}; dropping it`);
    return;
  }
  const route = SECTION_ROUTE[section.kind];
  // Navigating to the identical URL reloads the page, which is not what pressing the key means.
  if (url.hash === `#${route}`) return;
  url.hash = route;
  await chrome.tabs.update(tab, { url: url.toString() });
}
```

Reading the tab's URL is what the `tabs` permission is already there for, and navigating an existing tab with `chrome.tabs.update` needs no host permission, so the manifest's hosts do not change.

## The worker routes

`src/background.ts` gains a message listener where `connect` builds the socket:

```ts
 async function connect(): Promise<WebSocket> {
   if (
     socket !== null &&
     (socket.readyState === WebSocket.OPEN ||
       socket.readyState === WebSocket.CONNECTING)
   ) {
     return socket;
   }
   const ws = new WebSocket(`ws://127.0.0.1:${String(await port())}`);
   const forget = (): void => {
     if (socket === ws) socket = null;
   };
   ws.addEventListener("close", forget);
   ws.addEventListener("error", forget);
+  ws.addEventListener("message", (ev: MessageEvent<string>) => {
+    const frame = parseFrame(ev.data);
+    if (frame !== null) void deliver(frame.value);
+  });
+  // mercury writes a command to the connection that reported the tab it names, so a fresh
+  // connection has to say what is front before it can receive anything for it.
+  ws.addEventListener("open", () => {
+    void pushFrontTab();
+  });
   socket = ws;
   return ws;
 }
```

and the routing itself:

```ts
/** Perform a command in the tab it names. */
async function deliver({ tab, command }: BrowserCommand): Promise<void> {
  switch (command.kind) {
    case "SiteCommand.ClaudeAi":
      await runClaudeAiCommand(tab, command.value);
      return;
  }
}

/** Report whatever tab is front, which is what a fresh connection owes mercury. */
async function pushFrontTab(): Promise<void> {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  await pushTab(tab);
}
```

A second report of a tab mercury already has is a no-op: `set_front_tab` writes the same value and dispatch produces no change.

## Staying reachable

The worker is evicted when it goes idle, and an evicted worker has no socket, so a command computed for its tab has nowhere to go. Socket activity keeps a live worker alive; what is needed on top of that is something to start a dead one.

An alarm, registered at the top level of the worker so it exists again after every restart:

```ts
const KEEPALIVE = "mercury-keepalive";

// 30 seconds is the shortest period Chrome accepts. Each firing starts the worker if it was
// evicted, and reconnecting reports the front tab, so mercury is writing to a live connection
// again without waiting for the next tab switch.
chrome.alarms.create(KEEPALIVE, { periodInMinutes: 0.5 });

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === KEEPALIVE) void connect();
});
```

`manifest.json`:

```json
-  "permissions": ["tabs", "storage"],
+  "permissions": ["alarms", "tabs", "storage"],
```

and the description follows what it now does:

```json
-  "description": "Pushes Chrome's active tab URL to a running mercury.",
+  "description": "Bridges Chrome and a running mercury: pushes the front tab, performs mercury's commands.",
```

## What is where

```
chrome-extension/src/
  background.ts        listeners, socket, routing, the keepalive alarm
  frames.ts            zod schemas over the generated types, and `parseFrame`
  options.ts           the port setting
  sites/claudeAi.ts    claude.ai's commands
  wire/                generated, checked in, never edited
```

`sites/` is one file per site level in mercury, so the split between them is the split `SiteCommand` already has.

## Changes

1. esbuild and the build scripts, with no behavior change: the extension does what it does today, bundled rather than transpiled.
2. zod, `frames.ts`, and the message listener, logging the command and doing nothing with it.
3. `sites/claudeAi.ts`, `deliver`, and the routing that reaches it.
4. The keepalive alarm, the `open` handler's report, and the manifest's `alarms`.

`README.md` gains the bundler in Build, `alarms` in what it asks for, and a second check under "Check that it works": pressing the key on a claude.ai tab opens settings.

## Verified

With mercury running and the extension reloaded:

- `pnpm build`, `pnpm typecheck`, `pnpm lint`, and `pnpm format:check` pass, which is what CI's `extension` job runs.
- On a `claude.ai` tab, `u` then `s` opens the settings page, and `~/Library/Logs/mercury/mercury.log` carries the dispatch that produced the command.
- The URL afterwards is the tab's own path with `#settings/general`, and the page did not reload: the site's router responded to the fragment.
- Pressing it twice does nothing the second time.
- With the worker's console open, a hand-written frame naming a tab that has navigated to another host logs the drop and leaves the tab alone.
- Leave Chrome untouched for two minutes, then press the key: the alarm's reconnect means it still works, with no tab switch in between.
