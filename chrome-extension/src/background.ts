import type { IncomingEvent } from "./wire/IncomingEvent";

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
 * Returns it rather than leaving the caller to read the module variable, which could by then hold
 * a different socket than the one it asked for.
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
    ws.addEventListener(
      "open",
      () => {
        ws.send(payload);
      },
      { once: true },
    );
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
