# messages from a Claude, and the HUD

The other half of `agentic-layer.md`. That doc sends a message to a running Claude; this one lets a Claude send messages back into freddie over the same socket, and surfaces them on a heads-up display.

A Claude reaches freddie the way the extension already does: a frame on the event socket. The socket's inbound path (`IncomingEvent`, `127.0.0.1:3883`, `external-events.md`) already exists and already deserializes one frame kind. This adds a second kind, a sender a Claude can invoke, and a HUD that shows what arrives.

## The inbound vocabulary

`IncomingEvent` gains an agent-message arm. Before, in `crates/mercury/src/external.rs`:

```rust
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
}
```

after:

```rust
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum IncomingEvent {
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
    #[serde(rename = "IncomingEvent.Agent")]
    Agent(AgentMessage),
}

/// A message a Claude sent into freddie.
#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct AgentMessage {
    /// Which Claude this is from: a label the sender chooses, shown on the HUD so two Claudes are
    /// told apart. A repo name, a task name, whatever the sender configured.
    pub from: String,
    pub body: AgentBody,
}

/// What the Claude is saying.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum AgentBody {
    /// The Claude produced a response: the text it wrote. Shown on the HUD, and this is the case
    /// the response config in this doc acts on.
    #[serde(rename = "AgentBody.Response")]
    Response(String),
    /// A short status, not a full response: "done", "needs input", "blocked". Shown briefly.
    #[serde(rename = "AgentBody.Notify")]
    Notify(String),
}
```

One frame on the wire:

```jsonc
// client -> freddie
{
  "kind": "IncomingEvent.Agent",
  "value": {
    "from": "freddie",
    "body": { "kind": "AgentBody.Response", "value": "Done. Tests pass." }
  }
}
```

`AgentBody` deserialize-only, like the rest of `IncomingEvent`: nothing freddie sends back out is spelled here.

## on_message routes it

`on_message` gains an arm. Before:

```rust
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            let _ = event_tx.send(tab(url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

after:

```rust
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            let _ = event_tx.send(tab(url));
        }
        Ok(IncomingEvent::Agent(msg)) => {
            debug!(from = %msg.from, "agent message");
            let _ = event_tx.send(agent_message(msg));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

with `agent_message` building `MercuryEvent::Agent(AgentMessage)`, beside `tab` in `state/mod.rs`.

## The HUD

`AgentBody::Response` and `AgentBody::Notify` land on a HUD: a translucent card of text, the overlay panel `freddie_overlay` already draws. The keymap overlay shows a compile-time `&'static str`; a HUD shows runtime text, so it cannot reuse `ShowOverlay(&'static str)`.

`freddie_overlay` already supports more than one panel (`crates/freddie_overlay/src/lib.rs`: "More than one overlay is fine"). The HUD is a second panel, independent of the keymap overlay, so an agent message and the layer keymap do not clobber each other. New effects:

```rust
pub enum MercuryEffect {
    // …existing…
    /// Put text on the HUD panel, replacing what it showed. Owned, because a HUD shows runtime
    /// text, not a compile-time keymap.
    ShowHud(String),
    /// Take the HUD down.
    HideHud,
}
```

`perform_effect` drives a second `OverlaySink`, the HUD's, exactly as it drives the keymap overlay's:

```rust
MercuryEffect::ShowHud(text) => hud.show(text),
MercuryEffect::HideHud => hud.hide(),
```

The HUD auto-dismisses. `ShowHud` is paired with a `Timer` that fires `HideHud` after the configured delay, and the timer's guard lives in the state that put the HUD up, so a newer message cancels the older one's dismissal:

```rust
pub struct Mercury {
    // …existing…
    /// The HUD's dismissal timer while a message is up, `None` when the HUD is down. Dropping it
    /// cancels the pending `HideHud`, so a message arriving while one is shown replaces it rather
    /// than letting the first one's timer blank the second.
    hud: Option<TimerGuard>,
}
```

## The response config

"Whenever a Claude responds, do something" is data, not a hardcoded arm. The action a message kind triggers is configured:

```rust
// crates/mercury/src/agentic.rs

/// What happens when an agent message arrives, per kind.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub on_response: HudAction,
    pub on_notify: HudAction,
}

/// How an arriving message reaches the HUD.
#[derive(Clone, Copy, Debug)]
pub enum HudAction {
    /// Show it for this long, then dismiss.
    Show { seconds: u64 },
    /// Drop it: log only, no HUD.
    Ignore,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            on_response: HudAction::Show { seconds: 8 },
            on_notify: HudAction::Show { seconds: 3 },
        }
    }
}
```

The handler for `MercuryEvent::Agent` reads `root.agent_config`, picks the action for the body's kind, and returns `ShowHud(rendered)` plus the dismissal `Timer` when the action is `Show`. `rendered` is `from` and the body text on the HUD, e.g. `freddie: Done. Tests pass.`.

Making the config a field, not constants, is what lets a later change drive it from a file or a key without touching the handler. It starts with `Default`.

## The sender

A Claude sends the frame with a WebSocket client, not a raw socket write and not from a web page: the socket does a WebSocket handshake and its origin gate refuses `http(s)://` origins (`external-events.md`), so a browser tab's `fetch` cannot reach it. A native client can.

The reference sender is a freddie CLI verb, which owns the handshake so a caller does not have to:

```
freddie notify --from freddie --response "Done. Tests pass."
freddie notify --from freddie --notify "needs input"
```

It opens a WebSocket to `127.0.0.1:3883`, sends one `IncomingEvent.Agent` frame, and exits. This is the same crate and the same client machinery `freddie-cli.md` builds for the lifecycle verbs; the frame types are shared with mercury through the wire module.

```rust
/// `freddie notify`: send one agent message to a running freddie and exit. The Claude-facing half
/// of the socket, spelled as a verb so a tool call or a hook can shell out to it.
struct Notify {
    from: String,
    body: AgentBody,
}
```

## The tool

A Claude invokes the sender one of two ways. Both are configuration outside this repo; freddie provides the verb and the frame, not the wiring.

- A Claude Code Stop hook that runs `freddie notify --from <name> --response "$(…)"` when a turn ends, so every response reaches the HUD with no tool call.
- A tool the model calls deliberately (an MCP tool, or a shell tool) that runs the same verb, so a Claude decides when to speak to freddie rather than doing it on every turn.

The hook is the zero-effort default; the tool is for a Claude that should notify selectively. Both reduce to running `freddie notify`.

## Changes

Ordered, each independently shippable.

1. The HUD: `ShowHud(String)`, `HideHud`, the second `OverlaySink`, the `hud` timer guard, and its dismissal. Shippable with a throwaway bind that shows text on it, before any agent message exists.
2. `IncomingEvent::Agent`, `AgentMessage`, `AgentBody`, the `on_message` arm, `MercuryEvent::Agent`, and a handler that shows the body on the HUD for a fixed time. End to end from a hand-sent frame.
3. `AgentConfig`, `HudAction`, the `agent_config` field, and the handler reading it instead of a fixed time.
4. `freddie notify`: the CLI verb that opens the socket and sends the frame. Depends on `freddie-cli.md`'s client machinery.
5. The hook and tool wiring: an example Stop hook and an example tool definition, checked in as documentation, pointing at `freddie notify`.

## Verified

Against a running freddie, changes 1–3 need one restart:

- `freddie notify --from test --response "hello"` (change 4), or a hand-built frame on `127.0.0.1:3883`, puts `test: hello` on the HUD, and it dismisses after the configured seconds.
- A second message while the first is up replaces it, and only one dismissal fires.
- A malformed agent frame is logged and dropped, and the socket keeps serving, as the Tab path already does.
