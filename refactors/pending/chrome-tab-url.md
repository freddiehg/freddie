# per-tab key remaps for Chrome

Not built. The goal: when Chrome's front tab is a site mercury knows, the in-app Chrome layer gains site-specific binds. Concretely, on `claude.ai`, `n` sends `cmd-shift-o` (new chat); on any other site that bind is gone. This is the app layer's derived-child trick, one level deeper: the app level keys off which app is frontmost, and a new site level under it keys off the front tab's URL.

Two halves, and the doc splits along them. v0 is pure key remapping: the model reads the URL and emits a keystroke, which is everything the current effect layer already does. The speculation section is about effects that do more than emit a keystroke, which needs a real channel into Chrome.

## v0: remap keys per site

Five pieces, four of them copies of machinery that already exists.

### 1. The URL lives at the root

The front tab's URL goes on `Mercury`, next to `foregrounded`:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    pub current_url: Option<String>,
    #[resolve_into]
    pub layer: Layer,
}
```

It has to live on a persistent node, and the root is the only one that qualifies. `ChromeApp` is a derived node, rebuilt from scratch on every dispatch, so it cannot hold state that outlives a dispatch (this is the same reason `foregrounded` is at the root, not on `AppLayer`). `Option`, because there may be no front tab, or Chrome may not be frontmost, or the source has not reported yet.

### 2. A source that reports the URL

`freddie_app_nav` reports which app is frontmost; nothing reports Chrome's active tab. That is a new source crate, `freddie_chrome_tab`, the tab analog of the app watcher. Its v0 is an osascript poll:

```
osascript -e 'tell application "Google Chrome" to get URL of active tab of front window'
```

Poll on an interval, and only while Chrome is the frontmost app (mercury already knows that from `foregrounded`, so the watcher does not query a backgrounded Chrome). Each read that differs from the last emits a `TabEvent { url }`, the same shape as the foreground watcher emitting `foreground(app)`. Polling is the crude version; the speculation section replaces it with a push source.

Reading another app over Apple Events needs the Automation entitlement, which the user grants once on first use.

### 3. The event and its handler

A new source in the model, mirroring `Quit` and `Foregrounded` exactly:

- `MercuryEvent::Tab(TabEvent { url })` and a `Tab` trigger.
- `Tab => on_tab` bound at the root; `on_tab` writes `url` into `current_url`. If Chrome stops being frontmost, a `Foreground` event to another app can clear it, or `on_tab` can carry `None`.

### 4. The dynamic child that matches the URL

`ChromeApp` gets its own derived child, keyed on the URL, the way `AppLayer`'s `app_data` is keyed on the app:

```rust
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
pub struct ChromeApp {}

/// Reads the root's current URL and builds the site's level from it.
fn site_data(path: &ChromeAppNode) -> Option<SiteData> {
    let url = path./* ascend to root */.current_url.as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite {})),
        Site::Other => None,
    }
}

#[derive(Bind, Debug)]
#[derived_node(parent = ChromeAppNode)]
#[binds(MercuryStruct)]
#[bind(Key::KeyN.down() => new_chat)]
pub struct ClaudeAiSite {}
```

A derived level can itself have a derived child (the `derived.rs` test pins exactly this: two derived levels, one under the other, reaching the root). So `ChromeApp` -> `SiteData` -> the site node is a legal chain, and `site_data` reaches `root.current_url` by ascending its parent chain the way `app_data` reaches `root.foregrounded`. A site with no binds is not a variant and returns `None`, exactly like `App::Zed` in `app_data`.

`Site::from_url` parses the host and maps it, the browser-tab analog of `App::from_bundle_id`: `claude.ai` -> `Site::ClaudeAi`, everything else -> `Site::Other`. Keep the raw URL on the root rather than resolving to `Site` at the source, so later handlers that want the URL itself (copy-url, open-in-editor) still have it.

### 5. The effect is just a keystroke

`new_chat` returns the chord that is already expressible today:

```rust
fn new_chat(_ev: &KeyEvent, _node: /* ClaudeAiSite node */) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Tap { modifiers: vec![Key::MetaLeft, Key::ShiftLeft], key: Key::KeyO }]
}
```

The effect loop emits `cmd-shift-o`, Chrome's own shortcut for a new chat fires, done. Nothing new in the effect layer: v0 is a remap, not an automation. That is the whole point of drawing the v0 line here. Every site bind that maps to a keystroke the site already understands works this way.

### The staleness gap

The recorded URL lags the real tab by up to one poll interval, the same shape as nav's `has_navigated` gap where `foregrounded` lags the real front app. A keystroke that lands in the gap resolves against the previous site. The poll cadence bounds it; a push source (below) closes it. If it bites before then, the `has_navigated` pattern carries over: a `tab_pending` flag set on a Chrome foreground and cleared by the first `TabEvent`, with `site_data` returning `None` while it is set.

## Speculation: effects that do more than emit a keystroke

v0 can only ask the site to do something it already binds to a key. Plenty of actions have no such key: "open this tab's URL in Zed", "run JavaScript in the page and read a value back", "close every tab matching a pattern", "read the current selection". Those need to drive Chrome directly, which means a new kind of effect and a channel to perform it on. Two channels, increasing in power and cost.

### osascript / Apple Events

`tell application "Google Chrome"` can get and set the active tab's URL, open and close tabs, enumerate windows, and execute JavaScript in a tab (`execute ... javascript "..."`, gated behind Chrome's "Allow JavaScript from Apple Events" toggle in the Develop menu). The model side is one new effect, say `MercuryEffect::Browser(BrowserCommand)`, performed by spawning `osascript` fire-and-forget on its own thread, the way `foreground_app` spawns `open`. This covers most "do something to the browser" needs with no extension.

Its ceiling: osascript spawn latency is tens of milliseconds, JavaScript-from-Apple-Events is a permission most users have off by default, and AppleScript's tab model is clumsy for anything structured. It is a fine action channel and a poor event source (you still have to poll it for tab changes).

### A custom Chrome extension + native messaging

The robust answer to both halves at once. A small extension talks to a native host binary over stdin/stdout JSON (Chrome native messaging). The extension pushes tab and URL changes in real time, so it is a better SOURCE than polling (no interval, no gap), and it can call privileged extension APIs (`tabs.create`, `tabs.query`, `scripting.executeScript`) that AppleScript cannot reach cleanly, so it is a better ACTION channel. The `TabEvent` source and the `Browser` effect both move onto this one pipe.

We do not have such an extension; this is the doc that would decide to build one. Its cost is real: shipping and installing an unpacked or store extension, registering the native-messaging manifest, and running a host process that mercury launches and supervises. It is the endgame, not the starting point.

The path: v0 is the osascript poll plus keystroke remaps. Add the `Browser` effect over osascript when the first action appears that no keystroke expresses. Build the extension only when we hit a wall osascript cannot clear, either push-latency tab events or an action the extension APIs alone can do; at that point it subsumes the osascript source and the osascript effect both.

## Open questions

- The root field's type: raw `String`, a parsed `Url`, or a resolved `Site`. Raw string keeps handlers' options open; `Url` gives structured host/path matching; `Site` is smallest but throws away the URL.
- Whether the poll gates on Chrome being frontmost (it should) and the cadence when it is.
- Whether this generalizes across browsers (Safari, Arc) behind one `TabEvent`, the way `App::from_bundle_id` collapses apps behind one enum.
- Permissions to smooth over on first run: Automation for Apple Events, the JavaScript-from-Apple-Events toggle for `execute javascript`, and extension install for the native-messaging path.
