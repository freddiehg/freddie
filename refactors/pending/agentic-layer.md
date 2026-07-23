# the agentic layer

A layer whose keys deliver something to a running Claude: a dictation, a copied selection, a stashed snippet, or a screenshot. Every delivery carries a context envelope describing where it came from, so the Claude on the other end knows what app, site, and tab were in front when the key was pressed.

This is the freddie → Claude direction. The Claude → freddie direction (a Claude sending messages back, and freddie surfacing them on a HUD) is `agent-to-freddie.md`.

The layer collides in name with mercury's `agent.rs`, which is the launchd launch agent and nothing to do with a Claude. The launch agent keeps the name `Agent`; the destination this layer speaks to is a `Claude`.

## What a Claude is

A destination where a running Claude receives text or an image. Three kinds, which is the identity scheme `effects-and-events.md` calls the enabling item:

```rust
// crates/mercury/src/agentic.rs

/// A place a running Claude can be handed a message.
#[derive(Clone, Debug)]
pub enum Claude {
    /// A claude.ai tab, addressed over the event socket by its `TabId`. Delivery is a socket
    /// command, so nothing has to be focused first.
    Tab(TabId),
    /// A Claude Code in a tmux pane, addressed by `tmux send-keys -t <target>`. Exact, and needs
    /// no focus.
    Pane(PaneTarget),
    /// A Claude in a terminal or desktop window with no address of its own. Delivery is focus the
    /// window, then paste, with the race that implies.
    Window(App),
}

/// A tmux pane address: `session:window.pane`, as `tmux send-keys -t` wants it.
#[derive(Clone, Debug)]
pub struct PaneTarget {
    pub target: String,
}
```

`Tab` depends on `external-effects.md`, which introduces `TabId` and the socket write-back that carries a command to the tab that reported it. Until that lands, the tractable kinds are `Pane` and `Window`.

## The message

A message is a payload plus the context envelope. The envelope is a pure function of state, assembled at the moment the key is pressed, so it names what was in front then rather than what is in front when the (possibly async) delivery completes.

```rust
// crates/mercury/src/agentic.rs

/// Everything a delivery carries: what to send, and the context it was sent from.
#[derive(Debug)]
pub struct Message {
    pub context: Context,
    pub payload: Payload,
}

/// Where the message came from, read out of the model. Rendered ahead of the payload so the Claude
/// sees its provenance before its content.
#[derive(Debug)]
pub struct Context {
    /// The front app, stale-tolerant: `foreground.app()`, not `confirmed()`, because a delivery
    /// fired mid-nav still describes where the key was pressed.
    pub app: App,
    /// The site in the front Chrome tab, `None` off Chrome or before the tab source reports.
    pub site: Option<Site>,
    /// The front tab's URL, same source as `site`.
    pub url: Option<String>,
    /// The layer the key was pressed in: `layer().name()`.
    pub layer: &'static str,
}

/// What is being delivered.
#[derive(Debug)]
pub enum Payload {
    /// Text: a copied selection, the pinboard's contents, a dictation once Wispr has produced it.
    Text(String),
    /// An image on disk: a screenshot's file, for a destination that takes a path rather than a
    /// paste.
    Image(PathBuf),
    /// Dictation that has not happened yet. The payload is empty; delivery focuses the target and
    /// triggers Wispr, which types into it directly. See `Trigger::Dictate`.
    Dictation,
}
```

`Context` renders to a fenced block prepended to a text payload:

```
<freddie-context>
app: Chrome
site: claude.ai
url: https://claude.ai/chat/6f1e...
layer: agent
</freddie-context>

<payload>
```

```rust
impl Context {
    /// The envelope as the Claude receives it, ahead of the payload.
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = String::from("<freddie-context>\n");
        writeln!(s, "app: {}", self.app.bundle_id()).ok();
        if let Some(site) = self.site {
            writeln!(s, "site: {site}").ok();
        }
        if let Some(url) = &self.url {
            writeln!(s, "url: {url}").ok();
        }
        writeln!(s, "layer: {}", self.layer).ok();
        s.push_str("</freddie-context>\n\n");
        s
    }

    /// Read the envelope out of the root.
    #[must_use]
    pub fn from_root(root: &Mercury) -> Self {
        let chrome = root.foreground.confirmed_chrome();
        Self {
            app: root.foreground.app(),
            site: chrome
                .and_then(|c| c.url.as_deref())
                .map(Site::from_url),
            url: chrome.and_then(|c| c.url.clone()),
            layer: root.layer().name(),
        }
    }
}
```

## The four triggers

Each trigger produces a `Payload`. Delivery is the same for all four once the payload exists.

```rust
/// What a key in the agentic layer does: produce a payload for the target.
#[derive(Clone, Copy, Debug)]
pub enum Trigger {
    /// Trigger Wispr Flow at the target and let it dictate into it. Emits `F19`, which Wispr
    /// listens for the way voicemode's `RCtrl+y` did. The payload is `Dictation`: there is no text
    /// yet, so delivery focuses the target, pastes the context envelope, then taps `F19` so the
    /// spoken words land after it.
    Dictate,
    /// Copy the current selection (`cmd-c`) and send it. The clipboard is read after the copy, so
    /// this is a state machine, not one effect: `cmd-c`, then a clipboard-read event carries the
    /// text back, then delivery.
    CopyAndSend,
    /// Send the pinboard's contents. No copy, no clipboard round-trip: the text is already in the
    /// model. Empty pinboard sends nothing.
    SendPinboard,
    /// Screenshot and send. `region` chooses interactive-region versus whole-screen.
    Screenshot { region: bool },
}
```

Trigger to payload:

- `Dictate` — `Payload::Dictation`. Delivery taps `F19` at the focused target.
- `CopyAndSend` — emit `cmd-c`, then a clipboard read (a new effect + event, below) hands back `Payload::Text`.
- `SendPinboard` — `Payload::Text(pinboard.clone())`, straight from `root.pinboard`.
- `Screenshot { region }` — a `screencapture` effect writes a file, then a screenshot-taken event carries `Payload::Image(path)`.

## Delivery

Delivery depends on the kind of `Claude`:

- `Tab(id)` — a socket command carrying the rendered message. Builds on `external-effects.md`'s `Command`, with a new variant:

  ```rust
  pub enum Command {
      OpenClaudeSettings,
      /// Put text into the tab's prompt box. The extension writes it into the composer and,
      /// for a text payload, may submit it.
      Prompt(String),
  }
  ```

- `Pane(target)` — `tmux send-keys -t <target> -l <text>` then `tmux send-keys -t <target> Enter`. A new effect that shells out off the effect loop, the way `front_tab_url` already shells to `osascript`.

- `Window(app)` — put the text on the clipboard (`Copy(Copied::Text)`), `Foreground(app)`, and once the foreground event confirms the app is up, `Tap(cmd-v)`. Focus is async, so this is the pending-delivery state below.

An image payload has no text path: `Tab` pastes the image (the extension reads the file and pastes it into the composer), `Window` puts the image on the clipboard and pastes, `Pane` sends the file path as text since a terminal cannot take a pasted image.

### Focus is async, so delivery is a state

`Window` delivery cannot be one effect: the paste has to wait for the foreground to confirm, or it lands in whatever was front when the key was pressed. This is the shape `effects-and-events.md` describes for the cut-rewrite-paste flow and `overall-plan.md` prescribes for anything with a delay.

```rust
/// A delivery waiting on the target to come forward. Held in the agentic layer's state; the
/// foreground event that names `app` is what completes it.
#[derive(Debug)]
pub struct PendingDelivery {
    pub app: App,
    pub payload: Payload,
}
```

`Foreground(app)` is emitted, the pending delivery is stored, and when `Foreground`'s confirming event arrives naming the same app, the paste fires and the pending delivery clears. A foreground naming a different app abandons it rather than pasting into the wrong window.

## The two modes, unified

The layer holds an optional target and an optional staged payload:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(agent_target_chooser)]
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyD.down() => dictate,
    Key::KeyC.down() => copy_and_send,
    Key::KeyP.down() => send_pinboard,
    Key::KeyS.down() => screenshot_region,
    Key::KeyF.down() => screenshot_full,
)]
pub struct AgentLayer {
    pub(crate) home_timeout: TimerGuard,
    /// The Claude every trigger delivers to. `Some` is mode A: the target was chosen before the
    /// layer was entered (or defaulted to the last one), and a trigger delivers straight away.
    /// `None` is mode B: a trigger stages its payload and the chooser child resolves the target.
    pub(crate) target: Option<Claude>,
    /// A payload produced by a trigger while `target` was `None`, waiting for the chooser.
    pub(crate) staged: Option<Payload>,
}
```

- Mode A: `target` is `Some`. A trigger produces its payload and delivers to `target` immediately.
- Mode B: `target` is `None`. A trigger stores its payload in `staged`; the derived child `agent_target_chooser` is present exactly while `staged.is_some() && target.is_none()`, binding one key per known Claude. Choosing one sets `target` and delivers `staged`.

The chooser is a state-controlled child, `laserbeam-state-controlled-children.md`: it exists only when there is a staged payload and no target, so its keys are live precisely when a target is being asked for and gone otherwise.

Both modes are one layer, so nothing chooses between them at build time. Entering the layer with a known target (the common case: you were just talking to a Claude) is mode A; entering it cold and picking after is mode B.

## New effects

```rust
pub enum MercuryEffect {
    // …existing…
    /// Read the clipboard and hand it back as an event. For `CopyAndSend`, fired after the
    /// `cmd-c` tap. `arboard::Clipboard::get_text`, off the effect loop.
    ReadClipboard(ClipboardPurpose),
    /// Take a screenshot. `region` runs `screencapture -i`, else the whole screen; the file path
    /// comes back as an event.
    Screenshot(ScreenshotRequest),
    /// `tmux send-keys` to a pane. Shells out off the effect loop.
    TmuxSend(TmuxSend),
}

pub struct ScreenshotRequest {
    pub region: bool,
    pub dest: PathBuf,
}

pub struct TmuxSend {
    pub target: String,
    pub text: String,
}
```

`ReadClipboard` and `Screenshot` return their result as events (`ClipboardRead(String)`, `ScreenshotTaken(PathBuf)`), which the layer's handlers turn into a delivery. This keeps `state.handle` pure: the outside world (the clipboard, the screen) reaches a handler only as an event, per the handler rules in `CLAUDE.md`.

Triggering Wispr needs no new effect: it is `tap(Key::F19, ModifierFlags::empty())`.

## The pinboard

`SendPinboard` reads a buffer that does not exist yet:

```rust
pub struct Mercury {
    // …existing…
    /// A stashed snippet, set by a copy-to-pinboard action and pasted by the agentic layer or by
    /// a paste key. `None` until something is stashed.
    pub pinboard: Option<String>,
}
```

Setting it is a copy-to-pinboard action (from home, or a dedicated key): `cmd-c`, `ReadClipboard(Pinboard)`, and the `ClipboardRead` event writes `root.pinboard`. This is close to `ideas.md`'s clipboard-history entry, narrowed to a single slot.

## Changes

Ordered so each is independently shippable and the early ones are prefactors. The first three are small enough to be their own docs; split them out if they grow.

1. Trigger Wispr: a bind that emits `F19`. No new type, no new effect. Proves the F19 path end to end and is useful on its own.
2. The `Screenshot` effect and its `ScreenshotTaken` event, with a bind that screenshots to a file (region and full). Shippable as a screenshot key with no Claude involved.
3. The pinboard: the `pinboard` field, `ReadClipboard`, the `ClipboardRead` event, a copy-to-pinboard action, and a paste-pinboard key. Shippable as a snippet stash independent of any Claude.
4. `Claude`, `PaneTarget`, `Message`, `Context`, `Payload`, and `Context::from_root`/`render`. Types only, no binds.
5. Delivery: `TmuxSend`, the `Command::Prompt` extension of `external-effects.md`, `PendingDelivery`, and the foreground-confirm completion. This is the consequential change; 1–4 make it small.
6. `AgentLayer` with its binds and the `agent_target_chooser` child, wiring 1–5. The layer's entry point (which key from home enters it, and how a default target is chosen) is decided here.

## Open, to decide before change 4

- How a target is named for the chooser (change 6's child): the set of known Claudes has to come from somewhere. tmux panes running Claude Code can be enumerated (`tmux list-panes`); claude.ai tabs are the ones the extension has reported. Whether the chooser lists live-discovered targets or a configured set is undecided.
- Whether `Screenshot`'s file is cleaned up after delivery, or left for the user, and where it is written.
