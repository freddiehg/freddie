# external effects: one command frame to the extension

`external-events.md` shipped the direction where a client tells mercury something. This is the other direction: mercury tells the browser to do something, over the same socket.

Effects are fire and forget. `state.handle` returns `Vec<MercuryEffect>`, the effect loop performs each one, and no effect returns a value to the model. A command is one more effect, performed by writing a frame.

The vocabulary here is one flat enum for the whole process, and any handler can build any command in it. Scoping a command to the state that may emit it is `scoped-commands.md`, which lands on top of this. Performing the command in the browser is `extension-commands.md`.

Every command names the tab it is addressed to. The extension reports a tab id alongside the URL, mercury holds both, and delivery is to the connection that made that report. Nothing asks which tab is front at delivery time.

## Tabs are named

`TabMessage` carries the id, before:

```rust
#[derive(serde::Deserialize, Debug)]
pub struct TabMessage {
    pub url: String,
}
```

after:

```rust
/// Chrome's own tab id: unique across windows, stable for the life of the tab.
///
/// A newtype so a tab id cannot be passed where any other number is wanted. `i32` because that is
/// what `chrome.tabs` hands out, negatives included (`chrome.tabs.TAB_ID_NONE` is `-1`).
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize
)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct TabId(pub i32);

#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct TabMessage {
    pub tab: TabId,
    pub url: String,
}
```

`TabEvent` carries it too, in `sources.rs`:

```rust
 pub struct TabEvent {
+    pub tab: TabId,
     pub url: String,
 }
```

and its constructor in `state/mod.rs`, before:

```rust
#[must_use]
pub const fn tab(url: String) -> MercuryEvent {
    MercuryEvent::Tab(TabEvent { url })
}
```

after:

```rust
/// A tab event, carrying the front tab as the browser reported it.
#[must_use]
pub const fn tab(tab: TabId, url: String) -> MercuryEvent {
    MercuryEvent::Tab(TabEvent { tab, url })
}
```

## The front tab is one value

The id and the URL arrive together and are meaningless apart, so the state holds one value rather than two options that could disagree. Before:

```rust
pub struct ForegroundedChrome {
    pub url: Option<String>,
}
```

after:

```rust
pub struct ForegroundedChrome {
    /// The front tab, `None` until the tab source reports. A site level resolves only once this
    /// is `Some`, so a key pressed in the gap after Chrome comes up is unbound rather than aimed
    /// at whatever site was there before.
    pub tab: Option<FrontTab>,
}

/// The front tab as the browser last reported it. The id addresses a command; the URL decides
/// which site level resolves. Neither is useful without the other, so they are one value.
#[derive(Debug)]
pub struct FrontTab {
    pub id: TabId,
    pub url: String,
}
```

`ForegroundedApp::from_identity` builds `Chrome(ForegroundedChrome { tab: None })`.

`Foreground::set_tab_url` becomes `set_front_tab`, before:

```rust
pub fn set_tab_url(&mut self, url: String) {
    if self.navigating {
        return;
    }
    if let ForegroundedApp::Chrome(chrome) = &mut self.app {
        chrome.url = Some(url);
    }
}
```

after:

```rust
/// The tab source reported the front tab. Kept only while Chrome is the confirmed front app: a
/// report arriving while anything else is up describes a window nobody is looking at, and one
/// arriving mid-navigation belongs to the app being left.
pub fn set_front_tab(&mut self, tab: TabId, url: String) {
    if self.navigating {
        return;
    }
    if let ForegroundedApp::Chrome(chrome) = &mut self.app {
        chrome.tab = Some(FrontTab { id: tab, url });
    }
}
```

`record_tab_url` calls it:

```rust
-    root.foreground.set_tab_url(ev.url.clone());
+    root.foreground.set_front_tab(ev.tab, ev.url.clone());
```

The two readers of the URL follow the field. `site_data`:

```rust
-    let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
+    let url = &root.foreground.confirmed_chrome()?.tab.as_ref()?.url;
```

and `Layer::overlay_content`:

```rust
             Self::Site(_) => site::overlay_for(
                 foreground
                     .confirmed_chrome()
-                    .and_then(|chrome| chrome.url.as_deref())
-                    .map(Site::from_url),
+                    .and_then(|chrome| chrome.tab.as_ref())
+                    .map(|tab| Site::from_url(&tab.url)),
             ),
```

## The outgoing vocabulary

Serialize only, so nothing here can arrive from outside, exactly as `IncomingEvent` is deserialize only. The frame carries a `kind` at the top level, so the extension parses one shape whatever the payload is.

```rust
// crates/mercury/src/external.rs

/// Everything mercury may say to a connected client.
#[derive(serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum OutgoingEffect {
    #[serde(rename = "OutgoingEffect.Command")]
    Command(BrowserCommand),
}

/// A command and the tab it is addressed to.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct BrowserCommand {
    pub tab: TabId,
    pub command: Command,
}

/// What the browser is being asked to do.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum Command {
    /// Put claude.ai's settings up in the named tab.
    #[serde(rename = "Command.OpenClaudeSettings")]
    OpenClaudeSettings,
}
```

One frame on the wire:

```jsonc
// mercury -> client
{
  "kind": "OutgoingEffect.Command",
  "value": {
    "tab": 42,
    "command": { "kind": "Command.OpenClaudeSettings", "value": null }
  }
}
```

`TabId` is a newtype over `i32`, so serde writes it as a bare number and ts-rs exports `type TabId = number`.

## The effect

```rust
 pub enum MercuryEffect {
     Foreground(super::App),
     Tap(Chord),
     // …
+    /// Tell the connected browser to do something to a named tab.
+    Browser(BrowserCommand),
 }
```

## The socket writes back

`freddie_event_socket` hands the callback a handle to the connection the frame came from.

```rust
// crates/freddie_event_socket/src/lib.rs

/// A live client connection. Cloneable and cheap: it holds a sender into that connection's write
/// task, so a caller can keep one and write to it long after the frame that produced it.
#[derive(Clone)]
pub struct Client {
    outgoing: UnboundedSender<String>,
}

/// The connection has ended, which is what a client that went away looks like from here.
#[derive(Debug)]
pub struct Disconnected;

impl Client {
    /// Queue `text` for this client. It returns once the text is queued; the connection's write
    /// task performs the write.
    ///
    /// # Errors
    ///
    /// [`Disconnected`] if the connection has ended.
    pub fn send(&self, text: String) -> Result<(), Disconnected> {
        self.outgoing.send(text).map_err(|_| Disconnected)
    }
}
```

`listen`'s callback gains it, before:

```rust
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + Sync + 'static,
```

after:

```rust
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&Client, &str) + Send + Sync + 'static,
```

`serve` splits the socket, because the read loop and the write have to proceed independently:

```rust
async fn serve<F>(stream: TcpStream, on_message: Arc<F>, mut closed: watch::Receiver<()>)
where
    F: Fn(&Client, &str) + Send + Sync + 'static,
{
    let config = WebSocketConfig {
        max_message_size: Some(MAX_FRAME_BYTES),
        ..WebSocketConfig::default()
    };
    let ws = match accept_hdr_async_with_config(stream, check_origin, Some(config)).await {
        Ok(ws) => ws,
        Err(e) => {
            debug!(error = %e, "handshake failed");
            return;
        }
    };

    let (mut sink, mut incoming) = ws.split();
    let (outgoing_tx, mut outgoing_rx) = unbounded_channel::<String>();
    let client = Client { outgoing: outgoing_tx };

    loop {
        tokio::select! {
            // The socket was dropped. Say goodbye properly: dropping the sink here instead would
            // reset the connection, and the client would see a protocol error rather than a close.
            () = dropped(&mut closed) => {
                if let Err(e) = sink.close().await {
                    debug!(error = %e, "could not close cleanly");
                }
                break;
            }
            // The branch is disabled while the channel is empty, and the channel cannot close
            // while `client` is alive, which is for as long as this loop runs.
            Some(text) = outgoing_rx.recv() => {
                if let Err(e) = sink.send(Message::Text(text)).await {
                    debug!(error = %e, "could not write to a client");
                    break;
                }
            }
            frame = incoming.next() => match frame {
                Some(Ok(Message::Text(text))) => on_message(&client, text.as_str()),
                Some(Ok(Message::Binary(_))) => debug!("dropping a binary frame"),
                // Ping and Close are tungstenite's to answer, and it has already queued the reply.
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    debug!(error = %e, "connection ended");
                    break;
                }
                None => break,
            },
        }
    }
}
```

The write is unbounded and never blocks the caller, so `perform_effect` stays synchronous and the effect loop is never held up by a slow client.

## Delivery: the last report

```rust
// crates/mercury/src/external.rs

/// Where a command goes: the client that last reported a front tab, and which tab it reported.
///
/// One slot, not a map of every tab ever seen. Mercury addresses a command to the tab its state
/// says is front, and that belief comes from this report, so a command for any other tab was
/// computed against a report that has since been superseded. Dropping it is the safe end of that
/// race; delivering it would perform a command meant for one tab in another.
#[derive(Default)]
pub struct Browser {
    front: Mutex<Option<Reporter>>,
}

struct Reporter {
    tab: TabId,
    client: Client,
}

impl Browser {
    /// Record that `client` reported `tab` as the front tab.
    pub fn reported(&self, tab: TabId, client: &Client) {
        *self.lock() = Some(Reporter {
            tab,
            client: client.clone(),
        });
    }

    /// Write `command` to the client that reported its tab.
    ///
    /// Logged and dropped when the front tab is not the one addressed, when nothing has reported
    /// yet, and when that client has since disconnected. A command is fire and forget, so there is
    /// nothing to report back to the model in any of those cases.
    pub fn send(&self, command: BrowserCommand) {
        let tab = command.tab;
        let frame = match serde_json::to_string(&OutgoingEffect::Command(command)) {
            Ok(frame) => frame,
            Err(e) => {
                warn!(error = %e, "a command would not serialize");
                return;
            }
        };
        let guard = self.lock();
        let Some(reporter) = guard.as_ref() else {
            warn!(?tab, "no client has reported a tab");
            return;
        };
        if reporter.tab != tab {
            warn!(?tab, front = ?reporter.tab, "the command names a tab that is not front");
            return;
        }
        match reporter.client.send(frame) {
            Ok(()) => debug!(?tab, "command queued"),
            Err(Disconnected) => warn!(?tab, "the client that reported the tab is gone"),
        }
    }

    /// The guard, recovering from poisoning: the slot is written whole and read whole, so a panic
    /// elsewhere cannot leave it half updated.
    fn lock(&self) -> MutexGuard<'_, Option<Reporter>> {
        self.front.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
```

`on_message` records the sender, before:

```rust
pub fn on_message(text: &str, event_tx: &UnboundedSender<MercuryEvent>) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { url })) => {
            debug!(%url, "tab");
            let _ = event_tx.send(tab(url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

after:

```rust
pub fn on_message(
    client: &Client,
    text: &str,
    event_tx: &UnboundedSender<MercuryEvent>,
    browser: &Browser,
) {
    match serde_json::from_str::<IncomingEvent>(text) {
        Ok(IncomingEvent::Tab(TabMessage { tab: id, url })) => {
            debug!(?id, %url, "tab");
            browser.reported(id, client);
            let _ = event_tx.send(tab(id, url));
        }
        Err(e) => warn!(error = %e, frame = text, "undeserializable frame"),
    }
}
```

`run` owns one `Browser` and shares it with both halves:

```rust
     let browser = Arc::new(Browser::default());
     let _socket = freddie_event_socket::listen(port, {
         let event_tx = event_tx.clone();
-        move |text| mercury::on_message(text, &event_tx)
+        let browser = Arc::clone(&browser);
+        move |client, text| mercury::on_message(client, text, &event_tx, &browser)
     })
```

and `perform_effect` gains its arm, taking `browser: &Browser` alongside the emitter:

```rust
+        MercuryEffect::Browser(command) => browser.send(command),
```

## The bind

claude.ai's level binds `s`:

```rust
 #[derive(Bind, Debug)]
 #[derived_node(parent = SiteLayerPath)]
 #[binds(MercuryStruct)]
-#[bind(Key::KeyN.down() => new_chat)]
+#[bind(
+    Key::KeyN.down() => new_chat,
+    Key::KeyS.down() => open_settings,
+)]
 pub struct ClaudeAiSite {}
```

with the handler in `handlers/site.rs`, a new module beside the others in `handlers/mod.rs`:

```rust
//! The site levels' handlers.

use bind::Node;
use laserbeam::Ascend;

use crate::external::{BrowserCommand, Command};
use crate::state::{Mercury, MercuryPath};
use crate::{Key, MercuryEffect, ModifierFlags};
use crate::effect::tap;

/// `n` on claude.ai: start a new chat.
///
/// `cmd-shift-o` is the site's own shortcut, so this is a remap and not an automation: nothing has
/// to reach into the page.
pub(crate) fn new_chat<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyO, ModifierFlags::COMMAND | ModifierFlags::SHIFT)]
}

/// `s` on claude.ai: put settings up. The site has no keyboard shortcut for it, so this is the
/// first bind that asks the browser to act rather than emitting a keystroke.
///
/// Nothing is emitted when the front tab is unknown. The level resolved from a URL, so in practice
/// there is one, and `scoped-commands.md` moves the tab into the level and takes the case away.
pub(crate) fn open_settings<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    _ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    let root: &mut Mercury = node.parent.ascend();
    let Some(front) = root
        .foreground
        .confirmed_chrome()
        .and_then(|chrome| chrome.tab.as_ref())
    else {
        return Vec::new();
    };
    vec![MercuryEffect::Browser(BrowserCommand {
        tab: front.id,
        command: Command::OpenClaudeSettings,
    })]
}
```

`new_chat` moves here from `handlers/app.rs`, where it sits today; it is a site handler and not an in-app one.

`state/overlays/claude-ai.txt`:

```
  CLAUDE.AI
  ────────────────────
  n    new chat
  s    settings
  o    overlay
  t    typing
  esc  home
```

## The extension reports the id

`background.ts` sends what it already has. `pushUrl` becomes `pushTab`:

```ts
/**
 * Send the front tab to mercury.
 *
 * A tab that cannot be sent is dropped rather than queued: the next tab event supersedes it, so a
 * retry would deliver a stale answer to a question nobody asked yet.
 */
async function pushTab(tab: chrome.tabs.Tab | undefined): Promise<void> {
  if (tab?.id === undefined || tab.url === undefined || tab.url === "") return;
  const ws = await connect();
  const frame: IncomingEvent = {
    kind: "IncomingEvent.Tab",
    value: { tab: tab.id, url: tab.url },
  };
  // …unchanged from here: send when open, otherwise once on `open`.
}
```

and the three listeners hand it whole tabs:

```ts
chrome.tabs.onActivated.addListener(({ tabId }) => {
  void chrome.tabs.get(tabId).then(pushTab);
});

chrome.tabs.onUpdated.addListener((_tabId, info, tab) => {
  if (info.url !== undefined && tab.active) void pushTab(tab);
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) return;
  void chrome.tabs.query({ active: true, windowId }).then(([tab]) => pushTab(tab));
});
```

`onUpdated`'s `tab` carries the new URL in the same call, so `info.url` only decides whether this change was a navigation.

## Changes

Each is independently shippable, in order.

1. `Client`, `Disconnected`, and `listen`'s callback taking `&Client`. `freddie_event_socket`'s own tests are the only caller to update; the callback ignores the new argument.
2. `TabId`, the id on `TabMessage`, `TabEvent`, and the `tab` constructor; `FrontTab` and `ForegroundedChrome::tab`; `set_front_tab`; the two readers; the extension's push. Nothing sends a command yet, and the site layer behaves exactly as before.
3. `OutgoingEffect`, `BrowserCommand`, `Command`, `MercuryEffect::Browser`, `Browser`, `on_message`'s new arguments, and `perform_effect`'s arm.
4. The `s` bind, `handlers/site.rs`, and the overlay line.

## Tests

In `crates/mercury/tests/external.rs`, over a real connection on an OS-assigned port, the way the existing ones drive it:

- A client that reported tab 42 receives the exact frame above when `Browser::send` addresses 42.
- A command for a tab that is not the last reported one is not written to any client.
- A command sent before any client has reported is dropped, and the process is still serving afterwards.
- A command whose client has disconnected is dropped rather than panicking the socket's runtime.
- Two clients: the second's report replaces the first's, and a command for the first client's tab goes nowhere.
- `serde_json::to_string(&OutgoingEffect::Command(..))` is byte-for-byte the frame above.

In `crates/mercury/tests/transitions.rs`:

- `u` then `s` on a reported claude.ai tab produces exactly `MercuryEffect::Browser(BrowserCommand { tab, command: Command::OpenClaudeSettings })` carrying the reported id.
- `u` then `s` with Chrome front and no tab reported produces no effects, and no site level resolved.
- `u` then `s` on a reported tab whose URL is some other site produces no effects.
- A tab report while Ghostty is front leaves `ForegroundedChrome` untouched, as the URL-only version already asserts.

## Verified

Against the mercury that is already running, which needs no restart for the incoming half and one for the outgoing:

- Connect to `127.0.0.1:3883`, send a tab frame carrying an id, and read the dispatch record: the state carries `Chrome(ForegroundedChrome { tab: Some(FrontTab { id: TabId(42), url: "https://claude.ai/new" }) })`.
- Hold that same connection open, press `u` then `s`, and read the command frame off it.
