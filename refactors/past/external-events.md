# external events: a socket mercury can be told things on

mercury listens on a loopback WebSocket so processes outside it can push events in. `freddie_event_socket` owns the transport, `crates/mercury/src/external.rs` owns the vocabulary, and the Chrome extension (`chrome-extension.md`) is the first client. Mercury asking the outside world for something is the other direction and is still `external-effects.md`.

## What shipped

`crates/freddie_event_socket` (commit `038fd7d`, restored in `79487e9`), depending on `tokio`, `tokio-tungstenite` 0.24, `futures-util`, `http`, and `tracing`:

```rust
pub struct EventSocket;                                   // dropping it closes everything

pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + Sync + 'static;
```

A source in the mold of `freddie_app_nav`: it owns the listener, runs its accept and read loops off the main path, calls back per frame, and knows nothing about `MercuryEvent`, so figaro can take an event socket from the same call.

The bind is synchronous, through `std::net::TcpListener`, before the accept task is spawned. A busy port is therefore an `Err` from `listen` rather than a failure inside a task the caller would have to go looking for.

`EventSocket` holds the only `watch::Sender` and every task holds a receiver, so dropping it is what each of them is waiting on. Nothing is aborted by hand.

Per connection: the handshake is refused with 403 when the `Origin` is a web page's, only text frames reach `on_message`, binary frames are dropped with a `debug` line, and `max_message_size` is 64 KiB.

`crates/mercury/src/external.rs` (commit `3515d80`, restored in `79487e9`) holds `DEFAULT_PORT`, `IncomingEvent`, and `on_message`. `run` binds the socket above `freddie_keyboard::intercept`, so a refused start has not taken the keyboard yet, and a busy port panics naming the port and `lsof`.

## The vocabulary is its own type

`MercuryEvent` does not derive `Deserialize`. Deriving it would make `MercuryEvent::Key` and `MercuryEvent::Quit` constructible from the wire, so "no remote keyboard, no remote kill" would be a rule some match arm enforces rather than something the types say. `IncomingEvent` names exactly what an outside sender may say, and there is no arm to forget.

It shipped empty:

```rust
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {}
```

which compiles, and refuses every frame with ``unknown variant `IncomingEvent.Tab`, there are no variants``. That was the honest state of the transport before anything needed to be told something, and the `Ok(never) => match never {}` arm in `on_message` meant the first variant to land broke that line rather than being silently ignored. `chrome-tab-url.md`'s work added `Tab(TabMessage)` and the arm that sends it on `event_tx` (commit `bed246a`).

## The port

`127.0.0.1:3883`, from `--port` or `MERCURY_PORT`, resolved by clap (`mercury-cli.md`).

3883 is mercury's melting point, -38.83 °C, the one fact about the element. Below 49152, which is where macOS starts handing out ephemeral ports (`net.inet.ip.portrange.first`): a listener up there can find its port already taken by an outbound socket that grabbed it first. IANA has it registered to VRPN, which nothing here runs.

Loopback only, never `0.0.0.0`: a wildcard bind would let anything on the network tell mercury which app you are looking at.

A failed bind is fatal. A mercury that came up without its socket has per-site binds that silently do nothing, and no way to tell that apart from a broken extension without reading the log.

## No token, and web pages refused

Anything that reaches this socket can report which URL is frontmost and nothing else, so a hostile client buys one wrong chord the next time a site-bound key is pressed. A secret the user pastes into an options page costs more than that is worth. `external-effects.md` is where a connection becomes worth something and is the doc that adds the token.

Web pages are a different matter, and they are refused here. A WebSocket handshake is exempt from the same-origin policy: it crosses origins freely, the browser attaches `Origin`, and the server decides. Without the check, any page in any open tab could `new WebSocket("ws://127.0.0.1:3883")` and start sending frames. Chrome's Private Network Access work aims at this case, but its WebSocket enforcement has been partial and has shifted between releases, so nothing here depends on it.

The rule is a denylist. An `http`/`https` origin is a page and is refused; an absent one is a native client (a CLI, `websocat`, the test harness) and connects; anything else, in practice `chrome-extension://<id>`, connects. Demanding `chrome-extension://` would lock out the CLI and the test harness, which this exists to serve too. An `Origin` header that is not valid text is refused rather than treated as absent.

The extension's id is not matched. An unpacked development build's id follows from where it was loaded and a packed build's differs again, so matching one would break the install path `chrome-extension.md` describes, and what it would exclude is other extensions the user installed deliberately.

## What the tests pin

Seven in `freddie_event_socket`, six of them against a real `connect_async` client on an OS-assigned port so a running mercury is never disturbed:

- `origin_allowed` as a table, the one piece of pure logic here.
- A text frame arrives intact, including multibyte UTF-8.
- Two concurrent connections both deliver, and one closing leaves the other delivering.
- `Origin: https://evil.com` and `Origin: http://localhost:3000` are refused with 403 (a page served from loopback is still a page), `chrome-extension://abcdef` connects, and a refused handshake delivers nothing.
- A binary frame is dropped and the next text frame on that connection still arrives.
- A frame over 64 KiB closes only its own connection; the listener keeps accepting.
- Dropping the `EventSocket` ends the client's stream with a close, and frees the port for an immediate rebind on the same number.

That last one found a real defect. The first implementation dropped the `WebSocketStream` on shutdown, which resets the TCP connection, and the client saw `Protocol(ResetWithoutClosingHandshake)` rather than a close. `serve` now closes the stream before breaking. The assertion checks for the close specifically, because a reset also ends the stream and a looser assertion would have passed against the bug.

Four in mercury: a tab frame deserializes to its URL, nothing outside the vocabulary deserializes, a frame becomes an event on the channel, an unknown frame is dropped without disturbing the connection, a web page cannot connect through that composition, and `DEFAULT_PORT` is the number the extension hardcodes.

## Verified against the running process

With mercury up, by hand, no test harness:

- A handshake with no `Origin`: `101 Switching Protocols`.
- `Origin: https://evil.com`: `403 Forbidden`, body `origin not allowed`, never reaching the frame loop.
- `Origin: chrome-extension://abcdef`: connects, and frames arrive.
- `Origin: http://localhost:3000`: refused.
- `{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}` logged a dispatch whose state carried `Chrome(ForegroundedChrome { url: Some("https://claude.ai/new") })`, and garbage logged a parse error without closing the connection.

## What the workspace lints demanded

Three things that only showed up under `pedantic` and `nursery`, all in the pre-commit hook rather than in a plain `cargo clippy`:

- `check_origin` carries `#[expect(clippy::result_large_err)]`. `ErrorResponse` is 136 bytes and the signature is tungstenite's `Callback`, so there is nothing to box.
- `origin_allowed` is `is_none_or` rather than a `match`, per `option_if_let_else`.
- `WebSockets` unbackticked in a doc comment trips `doc_markdown`.

Reading a clippy run by piping it through `grep -E "^error"` finds nothing even when it failed: the output is ANSI-colored, so the pattern never matches at the start of a line. Check the exit status.
