# per-tab key remaps for Chrome

When Chrome's front tab is a site mercury knows, a layer of that site's binds resolves. On `claude.ai`, `n` sends `cmd-shift-o` and starts a new chat; on any other site that bind is not there. This is the app layer's derived-child trick one level deeper: the app level keys on which app is frontmost, and the site level keys on the front tab's URL.

Learning the URL is `chrome-extension.md`. It arrives here as an event off the socket (`refactors/past/external-events.md`), so the whole feature is drivable and testable without an extension.

## The URL lives inside the foregrounded Chrome

A `current_url: Option<String>` on `Mercury` would be meaningless whenever Chrome is not frontmost, and the type would not say so. Instead the foregrounded-app value carries its own per-app state, and the URL is Chrome's (commit `6ee919d`):

```rust
pub struct ForegroundedChrome {
    /// The front tab's URL, raw, as the tab source sent it.
    pub url: Option<String>,
}

pub enum ForegroundedApp {
    Chrome(ForegroundedChrome),
    Finder,
    Ghostty,
    Zed,
    #[default]
    Other,
}
```

`Foreground` holds a `ForegroundedApp` where it held an `App`. The URL exists in the type exactly when it can exist in fact, so there is no separate field to keep consistent and no "URL while Finder is front" state to be in.

`url: Option` is also the staleness gap, for free. Right after Chrome is foregrounded it is `None`, because the active tab is Chrome's to know and no app-activation event carries it, so the site level does not resolve until the source reports. A key pressed in that gap is unbound rather than aimed at the previous site.

### One enum or two

Two. `App` stays the `Copy` identity that events and effects speak, and `ForegroundedApp` is the stateful value at the root, with `identity()` and `from_identity()` between them.

The reason is not `Copy`, which references would have handled. It is that `MercuryEffect::Foreground(app)` means "bring this app up", and a URL field on it is meaningless in every case: an effect asking for an activation has no business naming a tab. `Foreground::app()` still returns `App`, so `overlay_content` and every existing test were untouched by the change.

### The URL's type

A raw `String`. `Site::from_url` matches a host, which is a scan of a short string even though the derived child runs on every dispatch, and the `url` crate pulls the ICU4X idna tree for a comparison `starts_with` covers. Keeping it raw also leaves the whole URL for handlers that want it, which a resolved `Site` would have discarded.

## The tab event

`TabEvent { url }` with a `Tabbed` trigger, beside the foreground source's pair, and `record_tab_url` bound at the root (commit `bed246a`). `IncomingEvent::Tab` off the socket becomes it.

`Foreground::set_tab_url` is where the dropping happens:

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

A URL arriving while anything else is up describes a window nobody is looking at, and one arriving mid-navigation belongs to the app being left. Neither is stored, so there is no stale URL to resolve a site level from later.

## `Site::from_url`

The browser-tab analog of `App::from_bundle_id`, in `sources.rs` beside it. The host has to match exactly, so `claude.ai.evil.com` is `Site::Other`: a suffix match would hand any domain that ends the right way whatever binds the real site has.

```rust
fn host(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let authority = after_scheme
        .find(['/', '?', '#'])
        .map_or(after_scheme, |end| &after_scheme[..end]);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host_port.find(':').map_or(host_port, |end| &host_port[..end]);
    (!host.is_empty()).then(|| host.strip_prefix("www.").unwrap_or(host))
}
```

Hand-rolled rather than the `url` crate, for the reason above. Chrome hands up a URL it has already normalized, so the host arrives lowercased and there is no case folding to do.

Two tests pin it. `https://claude.ai/new`, bare, with a query, with a fragment, with `www.`, with a port, and with userinfo all give `claude.ai`; `claude.ai.evil.com` and `notclaude.ai` give themselves; `chrome://extensions` gives `extensions`; `about:blank`, `file:///Users/x`, and `""` give `None`.

## The site layer, not a level under the in-app one

This is where what shipped diverges from the plan. The site's binds are their own layer, `u` from home, rather than a derived child of `ChromeApp` inside the in-app layer (commit `67ae544`).

The in-app layer is what the frontmost application can do, and it holds what is true of every tab; Chrome will grow more of those. The site layer is what the site in the front tab can do, and that changes as you move between tabs with no app switch at all. Folding the second into the first would have put two different keying rules in one layer, and `n` would have had to mean both "nav" and "new chat".

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
)]
pub struct SiteLayer {
    pub(crate) home_timeout: TimerGuard,
}

fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    // SiteLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite {})),
        Site::Other => None,
    }
}
```

`u` because it sits under the same finger as `i`, and the two are neighbours in meaning: `i` is the application, `u` is what it has open. The layer binds no `n` of its own, so `n` belongs to whichever site level resolved.

It stores no site. `site_data` reads the URL from the root on every dispatch, so switching tabs while sitting in the layer changes what is bound with no event of its own. `None` covers three cases a keypress cannot tell apart: Chrome is not the confirmed front app, the source has not reported, and the site has no binds.

The overlay is site-aware: `o` shows claude.ai's keymap when that is the front tab, and the bare layer's otherwise.

## The effect is a keystroke

```rust
pub(crate) fn new_chat<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyO, ModifierFlags::COMMAND | ModifierFlags::SHIFT)]
}
```

`cmd-shift-o` is claude.ai's own shortcut, so this is a remap and not an automation: nothing reaches into the page, and the command bus is not needed for it. Every site bind that maps to a keystroke the site already understands works this way.

`ModifierFlags` gained `BitOr` for it, so a chord's modifiers read as `COMMAND | SHIFT`. The modifiers ride as flags on the one key event rather than as synthetic presses around it, which is what keeps a modifier the user is really holding from being stranded, and `held` feeds the sync sweeps rather than the stamping, so the chord is exactly what the handler asked for whatever is held when it fires.

## Verified

The URL path, end to end against the running process: a frame on 3883 produced a dispatch whose state carried `Chrome(ForegroundedChrome { url: Some("https://claude.ai/new") })`, with no extension involved.

The keypress half was not driven. Pressing `u` then `n` on a claude.ai tab and seeing the `Tap` effect in the log is unconfirmed.

## Still open

Whether this generalizes across browsers behind one `TabEvent`, the way `App::from_bundle_id` collapses apps behind one enum. Only Chrome reports today.

`Site` is the name for now and is inaccurate: the layer keys on what the frontmost app has open, and Zed and the rest will want the same layer with a context of their own. That arrives as arms in `site_data` once their `ForegroundedApp` variants carry something, and the rename with them.
