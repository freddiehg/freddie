# a Chrome extension bridge for mercury

Not built. mercury needs to know Chrome's active-tab URL, for per-site key remaps (see `chrome-tab-url.md`), and eventually to drive Chrome directly (run page JavaScript, open and close tabs, read a selection). A Chrome extension is the bridge for both. It has a small v0 and a much larger eventual form, and they are worth separating: v0 is a few dozen lines, and the full thing is a real undertaking.

No polling, anywhere. The extension pushes; mercury never asks on a timer.

## v0: stream the active tab's URL

One direction, one message. A background service worker subscribes to `chrome.tabs.onActivated` (tab switch) and `chrome.tabs.onUpdated` (navigation within a tab), reads the active tab's URL, and sends it to mercury whenever it changes. That is the entire extension: two event listeners and one outbound `{ url }` message. It is event-driven, so mercury learns of a change the instant it happens.

mercury turns each message into a `TabEvent { url }`, and the model does the rest (`chrome-tab-url.md`). This is all v0 needs, because mercury's v0 only remaps keys, and a keystroke it emits itself.

### Transport

Two ways to carry the messages, and the choice matters because the eventual command bus rides the same channel.

- A local WebSocket: mercury runs a tiny loopback server, the extension's service worker connects to it and streams. It needs a host permission for the loopback origin and a way for both sides to agree on the port, but it is bidirectional from day one, so the command bus later adds message types rather than a new transport.
- Chrome native messaging: Chrome launches a native host binary from a manifest and pipes it stdin/stdout. It is awkward here because mercury is already running, so the native host would only be a relay from Chrome's short-lived host process into the running mercury.

For the one-way v0 stream either works; the WebSocket is the one that extends without rework.

## The complicated version: a command bus

The eventual form is bidirectional. mercury sends commands down (run this JavaScript in the tab, open a tab, close every tab matching a pattern, read the current selection); the extension runs them with the real extension APIs (`chrome.scripting.executeScript`, `chrome.tabs.query`, `chrome.tabs.create`) and sends results back up. This is where essentially all the complexity lives, and none of it is in the URL stream:

- Command registration and dispatch: a table of the command types the extension understands, each mapped to a handler. Adding an action means registering it on both sides and keeping them in step.
- Request/response correlation: commands that return a value need ids so a reply can be matched to its request, plus timeouts for commands that never answer.
- Errors and versioning: an action can fail in the page (a missing permission, a navigation mid-command), so the protocol needs an error shape, and a version so mercury and the extension degrade gracefully when one updates ahead of the other.
- Security: the loopback socket is reachable by any local process, and this channel can run arbitrary JavaScript in your tabs, so it needs at least a shared token, probably more.

Build the command bus only when a concrete action appears that no keystroke expresses, and keep the URL stream working without it until then.

The escape hatch before the bus exists is osascript, for a single scripted action fired from a bind: an on-keypress effect spawns `osascript` to run one AppleScript, including page JavaScript behind Chrome's "Allow JavaScript from Apple Events" toggle. That is event-driven (it runs on a keybind, not a timer), so it is not polling; it is just a slower, clumsier one-off than the bus, and it never touches the URL stream.

## Open questions

- Transport: WebSocket versus native messaging, and for the WebSocket the port and any handshake beyond being loopback-only.
- Install story: an unpacked dev load to start, a packed or store extension later.
- Whether one extension serves other Chromium browsers (Arc, Brave), and how Safari (no equivalent) is handled if at all.
- The command-bus protocol, when it is built: message framing, request ids, error shape, version, and auth token.
