# same-document navigation

The extension reports the front tab's URL from three listeners: `tabs.onActivated`, `tabs.onUpdated`, and `windows.onFocusChanged`. None of them fires when a page navigates through the History API, so a single-page app changes what it is showing and mercury keeps the URL the document loaded with.

This costs nothing while every bind keys on the host, because a route change inside a site leaves the host alone. It starts mattering the moment a bind keys on the path, which `github-site.md` does: `github.com` and `github.com/owner/repo` are one document on GitHub, and today mercury sees only whichever one you landed on.

Two changes, each independently shippable.

## Change 1: report history navigations

`chrome.webNavigation.onHistoryStateUpdated` fires on `pushState` and `replaceState`. `onReferenceFragmentUpdated` fires on a fragment change, which is a different URL and so a different route as far as `Site::from_url` is concerned. Both need the `webNavigation` permission.

`chrome-extension/manifest.json`, before:

```json
  "permissions": ["tabs", "storage"],
```

after:

```json
  "permissions": ["tabs", "storage", "webNavigation"],
```

`chrome-extension/src/background.ts`, after the `windows.onFocusChanged` listener:

```ts
/**
 * A same-document navigation in the front tab: `pushState`, `replaceState`, or a fragment change.
 *
 * `onUpdated` covers document loads and nothing else, so without these a single-page app changes
 * route with no event at all. `frameId === 0` keeps this to the top frame: an iframe navigating
 * does not change what the tab is showing.
 */
const onSameDocument = ({
  tabId,
  frameId,
  url,
}: chrome.webNavigation.WebNavigationTransitionCallbackDetails): void => {
  if (frameId !== 0) return;
  void chrome.tabs.get(tabId).then((tab) => {
    if (tab.active) void pushUrl(url);
  });
};

chrome.webNavigation.onHistoryStateUpdated.addListener(onSameDocument);
chrome.webNavigation.onReferenceFragmentUpdated.addListener(onSameDocument);
```

`onReferenceFragmentUpdated` hands the same details type, so one function serves both.

## Change 2: send a URL once

One page load already produces several identical frames, because `onUpdated`, `onActivated`, and `onFocusChanged` all see the same tab settle. Change 1 adds two more sources of the same duplicate. Every one of them dispatches through the model, and each is a log line.

The URL mercury holds is the last one sent, so a frame carrying what was already sent changes nothing and can be dropped at the source.

`chrome-extension/src/background.ts`, before:

```ts
let socket: WebSocket | null = null;
```

after:

```ts
let socket: WebSocket | null = null;

/**
 * The URL last sent on `socket`, so an identical one is not sent again.
 *
 * Cleared whenever the socket is dropped: a fresh mercury has never been told anything, and the
 * front tab's URL has to reach it even though this side already sent it to the last one.
 */
let lastSent: string | null = null;
```

`connect`'s `forget`, before:

```ts
  const forget = (): void => {
    if (socket === ws) socket = null;
  };
```

after:

```ts
  const forget = (): void => {
    if (socket !== ws) return;
    socket = null;
    lastSent = null;
  };
```

`pushUrl`, before:

```ts
async function pushUrl(url: string | undefined): Promise<void> {
  if (url === undefined || url === "") return;
  const ws = await connect();
```

after:

```ts
async function pushUrl(url: string | undefined): Promise<void> {
  if (url === undefined || url === "") return;
  if (url === lastSent) return;
  const ws = await connect();
  lastSent = url;
```

`lastSent` is set before the send rather than after, because the send may be deferred to the socket's `open` and a second call would otherwise get past the check while the first is still waiting. A send that then fails clears it through `forget`, which is the same path a dropped socket takes.
