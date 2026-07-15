# per-tab key remaps for Chrome

Not built. The goal: when Chrome's front tab is a site mercury knows, the in-app Chrome layer gains site-specific binds. Concretely, on `claude.ai`, `n` sends `cmd-shift-o` (new chat); on any other site that bind is gone. This is the app layer's derived-child trick, one level deeper: the app level keys off which app is frontmost, and a new site level under it keys off the front tab's URL.

This doc is the mercury side: how a per-site bind is expressed in the model. Learning the URL in the first place is the extension's job, in `chrome-extension.md`; here the URL is assumed to arrive as an event.

## The URL lives inside the foregrounded Chrome, not on the root

The obvious move is a `current_url: Option<String>` field on `Mercury`, but that field is meaningless whenever Chrome is not frontmost, and the type would not say so. Better: the foregrounded-app value carries its own per-app state, and the URL is Chrome's:

```rust
pub struct Mercury {
    pub foregrounded: ForegroundedApp,
    pub has_navigated: bool,
    #[resolve_into]
    pub layer: Layer,
}

pub enum ForegroundedApp {
    Chrome(ForegroundedChrome),
    Finder,
    Ghostty,
    Zed,
    Other,
}

pub struct ForegroundedChrome {
    /// `None` until the tab source reports; a site level resolves only once it is `Some`.
    pub url: Option<String>,
}
```

This renames today's `App` to `ForegroundedApp` and lets Chrome hold data. The URL exists in the type exactly when it can exist in fact, so there is no separate field to keep consistent and no "URL while Finder is front" nonsense state.

Identity versus state. The app-nav source still reports a plain identity (which app is frontmost, from a bundle id), and the foreground effect still asks for one; neither carries a URL. So keep a small `Copy` identity enum for events and effects (`ForegroundEvent { app }`, `MercuryEffect::Foreground(app)`, `App::from_bundle_id`), and build the stateful `ForegroundedApp` from it at the root. `on_foregrounded` sets `foregrounded = ForegroundedApp::Chrome(ForegroundedChrome { url: None })` when Chrome comes up; `on_tab` fills the `url` in place when a tab event arrives and `foregrounded` is already Chrome, and ignores it otherwise. (This is the one real design fork: one enum used everywhere, which drops `Copy` from events and effects, versus a `Copy` identity plus the stateful root value. The two-type split keeps events and effects trivial, so start there.)

It also folds in the staleness gap for free. `url: Option` is the same "we do not know yet" that nav's `has_navigated` encodes: right after Chrome is foregrounded, `url` is `None`, so the site level does not resolve until the source reports. No separate pending flag, and a key pressed in the gap is unbound rather than aimed at the previous site.

## The site derived child

`ChromeApp` gets its own derived child, keyed on the URL, the way `AppLayer`'s `app_data` is keyed on the app:

```rust
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
pub struct ChromeApp {}

/// Reads the foregrounded Chrome's URL and builds the site's level from it.
fn site_data(path: &ChromeAppNode) -> Option<SiteData> {
    let ForegroundedApp::Chrome(chrome) = &/* ascend to root */.foregrounded else {
        return None;
    };
    let url = chrome.url.as_deref()?; // None until the source reports: no site level yet
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

A derived level can itself have a derived child (the `derived.rs` test pins exactly this: two derived levels, one under the other, reaching the root). So `ChromeApp` -> `SiteData` -> the site node is a legal chain, and `site_data` reaches the root by ascending its parent chain the way `app_data` reaches `root.foregrounded`. A site with no binds is not a variant and returns `None`, exactly like `App::Zed` in `app_data`.

`Site::from_url` parses the host and maps it, the browser-tab analog of `App::from_bundle_id`: `claude.ai` -> `Site::ClaudeAi`, everything else -> `Site::Other`. The raw URL stays on the Chrome value, so later handlers that want the URL itself (copy-url, open-in-editor) still have it.

## The effect is just a keystroke

`new_chat` returns a chord that is already expressible today:

```rust
fn new_chat(_ev: &KeyEvent, _node: /* ClaudeAiSite node */) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Tap { modifiers: vec![Key::MetaLeft, Key::ShiftLeft], key: Key::KeyO }]
}
```

The effect loop emits `cmd-shift-o`, Chrome's own new-chat shortcut fires, done. Nothing new in the effect layer: this is a remap, not an automation. Every site bind that maps to a keystroke the site already understands works this way, and it is why v0 needs only the URL stream from the extension, not the command bus.

## The source is the extension's URL stream

Learning the URL is `chrome-extension.md`. Its v0 is a small extension that pushes the active tab's URL to mercury on every tab switch and navigation; mercury receives each as a `TabEvent { url }` handled by an `on_tab` at the root, the same shape as the foreground watcher emitting `foreground(app)`. Pushed, never polled.

Actions that no keystroke expresses (open the URL in Zed, run page JavaScript, close matching tabs) are the extension's command bus, the larger half of that doc. They are out of scope here: this doc stops at remapping keys.

## Open questions

- One enum or two for the foregrounded app: a single `ForegroundedApp` used by events and effects too (which loses `Copy`), or a `Copy` identity plus the stateful `ForegroundedApp` at the root. The two-type split is the starting recommendation.
- The URL's type on `ForegroundedChrome`: raw `String`, a parsed `Url` for structured host/path matching, or a resolved `Site` (smallest, but discards the URL that copy-url and friends want).
- How `Site::from_url` matches: exact host, host suffix, or path too (some sites want per-path binds).
- Whether this generalizes across browsers behind one `TabEvent`, the way `App::from_bundle_id` collapses apps behind one enum.
