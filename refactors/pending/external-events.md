# accepting external events (a socket source)

Not built. How mercury might take events from outside the process, so things other than the keyboard and the app watcher can drive it. The Chrome extension is the case that makes this concrete, and probably the thing that forces it.

## The idea

mercury's event loop already reads `MercuryEvent`s off a channel; it does not care where they came from. Today the senders are all in-process: the keyboard tap, the `freddie_app_nav` watcher, and the menu bar (Quit, Toggle). Nothing stops another sender from being a socket: a task that owns a local socket, accepts connections, deserializes each message into a `MercuryEvent`, and pushes it onto the same `event_tx` every other source uses. From the model's side it is just more events; the source is a socket rather than a `CGEventTap`.

That is the whole mechanism, and it generalizes "source" from "a device the OS gives us" to "anything that can connect and speak the event vocabulary."

## Why: the extension needs it

`chrome-extension.md`'s extension has to deliver the active tab's URL to mercury. The extension is a browser process; it cannot call into mercury directly, so it needs a wire, and a local socket is that wire. The extension pushes `{ url }`, the socket source turns it into `TabEvent { url }`, and it lands on `event_tx` like any other event. So "accept external events on a socket" is not a separate feature from the extension; it is the transport the extension rides, and building it is likely a prerequisite for the extension rather than a nice-to-have.

Once it exists, the same door serves scripting (a CLI that pokes mercury), voice mode, testing (drive the model from outside without a keyboard), and remote control.

## The shape

A source, in the mold of `freddie_app_nav`: it owns the socket, runs its accept/read loop off the main path, and calls a callback (or sends on a channel) for each message, the way `watch` calls back per activation. The binary wires that callback to `event_tx`. Dropping the source closes the socket.

Keep the transport generic and let mercury own the vocabulary, the same split the app watcher uses (it hands up a bundle-id string; mercury maps it to `App`). The socket crate hands up raw messages; mercury maps each to a `MercuryEvent`. So the crate is a `freddie_*` source (figaro would want an event socket identically), and the message-to-event mapping is mercury's, next to its event types.

## Transport

Two options, and the client decides:

- A Unix domain socket: simplest for a local CLI or script, file-based, no port. But a browser extension cannot open one.
- A localhost WebSocket (or plain TCP): what a Chrome extension can actually connect to (`chrome-extension.md` leans this way for exactly this reason). Costs a port and a handshake.

If the extension is the first real client, the WebSocket is the one to build, and it doubles as the bidirectional channel `chrome-extension.md` wants (events up, commands and results down) rather than a one-way event feed.

## Which events, and security

This is the part to get right, because injecting events is powerful. A `TabEvent` or `Foreground` or `Toggle` from outside is benign. A `Key` event from outside is remote keyboard control, and a `Quit` from outside kills the process. So the external vocabulary should probably be a restricted subset, or at least a deliberate whitelist, not "any `MercuryEvent`."

And the socket is a local attack surface: any process on the machine could connect and drive mercury. Loopback-only is the floor; a shared token the client must present is the sane next step, since the channel can move the focus, foreground apps, and (if allowed) synthesize keys.

## Open questions

- Transport: Unix socket, localhost WebSocket, or both behind one source trait. The extension pushes toward WebSocket; a CLI is happier with a Unix socket.
- The external event subset: which `MercuryEvent`s an outside sender may inject, and whether `Key` injection is ever allowed.
- Auth: loopback-only versus a token, and how a browser extension and a CLI each present it.
- One-way (events in) versus bidirectional (events in, effects and results out), and whether to unify with the extension command bus from the start.
- Serialization: JSON (extension-friendly, human-debuggable) versus something tighter; JSON is the obvious default.
- Whether this is one `freddie_*` crate with mercury owning the message-to-event mapping, mirroring `freddie_app_nav`.
