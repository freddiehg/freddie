# `Mercury.foregrounded: Option<ForegroundedApp>`; drop `Foreground`

A nav choice clears the front app until the watcher reports the one that actually came up. That gap is `None` on the root field. There is no `Foreground` struct, no `navigating` bool, and no `confirmed_*` readers: the `Option` is the state.

## Shape

Before, on `Mercury`:

```rust
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    // ...
}

/// While `navigating`, `app` is the PREVIOUS app: a nav choice foregrounded a new one, but the
/// watcher has not reported it yet, so the in-app level binds nothing until it does (see
/// [`app_data`]). The fields are private; the handlers drive it through the methods below.
#[derive(Debug)]
pub struct Foreground {
    app: ForegroundedApp,
    navigating: bool,
}
```

After:

```rust
pub struct Mercury {
    /// The frontmost app the watcher has reported.
    ///
    /// `None` while a nav choice has foregrounded an app the watcher has not yet reported: the
    /// in-app level binds nothing until it does (see [`app_data`]).
    pub foregrounded: Option<ForegroundedApp>,
    // ...
}
```

`struct Foreground` is deleted. Its methods are deleted. Call sites match or assign the field.

`ForegroundedApp` and `ForegroundedChrome` are unchanged:

```rust
#[derive(Debug, Default)]
pub struct ForegroundedChrome {
    pub url: Option<String>,
}

#[derive(Debug, Default)]
pub enum ForegroundedApp {
    Chrome(ForegroundedChrome),
    Finder,
    Ghostty,
    Zed,
    #[default]
    Other,
}

impl ForegroundedApp {
    #[must_use]
    pub const fn identity(&self) -> App { /* unchanged */ }

    #[must_use]
    pub const fn from_identity(app: App) -> Self { /* unchanged */ }
}
```

## Construction

Before:

```rust
pub fn new(front_app: App, windows: Windows) -> Self {
    Self {
        foreground: Foreground::new(front_app),
        windows,
        typing_state: TypingState::default(),
        overlay: None,
        layer: Self::boot_layer(),
    }
}
```

After:

```rust
pub fn new(front_app: App, windows: Windows) -> Self {
    Self {
        foregrounded: Some(ForegroundedApp::from_identity(front_app)),
        windows,
        typing_state: TypingState::default(),
        overlay: None,
        layer: Self::boot_layer(),
    }
}
```

Still no `Default` on `Mercury`: construction always seeds from the OS (see `seed-at-construction`).

## Handlers: assign the field

### Nav start (`handlers/nav.rs`)

Before:

```rust
root.foreground.start_navigating();
```

After:

```rust
root.foregrounded = None;
```

### Watcher report (`handlers/foreground.rs`)

Before:

```rust
root.foreground.set_front_app(ev.app);
```

After:

```rust
root.foregrounded = Some(ForegroundedApp::from_identity(ev.app));
```

Doc comment on `record_front_app`: it records the app the watcher reported; a pending nav ends because the field is filled, not because a flag clears.

### Tab URL (`handlers/tab.rs`)

Before:

```rust
root.foreground.set_tab_url(ev.url.clone());
```

After:

```rust
if let Some(ForegroundedApp::Chrome(chrome)) = &mut root.foregrounded {
    chrome.url = Some(ev.url.clone());
}
```

A URL while anything other than Chrome is up, or while `foregrounded` is `None`, is dropped by the match. No separate mid-nav check.

## Readers: match the field

### `app_data` (`state/app.rs`)

Before:

```rust
match root.foreground.confirmed() {
    Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
    Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
    _ => None,
}
```

After:

```rust
match root.foregrounded.as_ref().map(ForegroundedApp::identity) {
    Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
    Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
    _ => None,
}
```

Or match the enum directly:

```rust
match &root.foregrounded {
    Some(ForegroundedApp::Chrome(_)) => Some(AppData::Chrome(ChromeApp::new())),
    Some(ForegroundedApp::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
    _ => None,
}
```

Prefer the second: no detour through `App`.

### Copy URL (`handlers/app.rs`)

Before:

```rust
let Some(url) = root
    .foreground
    .confirmed_chrome()
    .and_then(|chrome| chrome.url.as_deref())
else {
    return vec![MercuryEffect::Copy(Copied::FrontTabUrl(part))];
};
```

After:

```rust
let Some(url) = root.foregrounded.as_ref().and_then(|app| match app {
    ForegroundedApp::Chrome(chrome) => chrome.url.as_deref(),
    _ => None,
}) else {
    return vec![MercuryEffect::Copy(Copied::FrontTabUrl(part))];
};
```

### `site_data` (`state/site.rs`)

Before:

```rust
let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
```

After:

```rust
let url = match &root.foregrounded {
    Some(ForegroundedApp::Chrome(chrome)) => chrome.url.as_deref()?,
    _ => return None,
};
```

### Overlay (`Layer::overlay_content`)

Before:

```rust
pub fn overlay_content(&self, foreground: &Foreground) -> &'static str {
    match self {
        // ...
        Self::InApp(_) => app::overlay_for(foreground.app()),
        Self::Site(_) => site::overlay_for(
            foreground
                .confirmed_chrome()
                .and_then(|chrome| chrome.url.as_deref())
                .map(Site::from_url),
        ),
        // ...
    }
}
```

After: take `&Option<ForegroundedApp>` (or `&Mercury` / the field by ref at the call site). Mid-nav is `None`; show the bare in-app keymap.

```rust
pub fn overlay_content(&self, foregrounded: &Option<ForegroundedApp>) -> &'static str {
    match self {
        Self::Home(_) => home::OVERLAY,
        Self::Nav(_) => nav::OVERLAY,
        Self::Resize(_) => resize::OVERLAY,
        Self::InApp(_) => match foregrounded {
            Some(app) => app::overlay_for(app.identity()),
            None => app::INAPP_OVERLAY,
        },
        Self::Site(_) => {
            let url = match foregrounded {
                Some(ForegroundedApp::Chrome(chrome)) => chrome.url.as_deref(),
                _ => None,
            };
            site::overlay_for(url.map(Site::from_url))
        }
        Self::Typing(_) => typing::OVERLAY,
    }
}
```

Call site on `Mercury` (toggle overlay):

```rust
// before
let content = self.layer.overlay_content(&self.foreground);

// after
let content = self.layer.overlay_content(&self.foregrounded);
```

`INAPP_OVERLAY` is already the fallback for Finder / Zed / Other. A pending nav is the same: the layer is up, no app-specific keymap is known.

## Exports

`lib.rs` drops `Foreground` from the `state` re-export. `ForegroundedApp` and `ForegroundedChrome` stay.

## Tests (`crates/mercury/tests/transitions.rs`)

Field path is `m.foregrounded`. Values are `Option<ForegroundedApp>`.

Pending-nav gap (today asserts old app still present and `navigating`):

```rust
// before
assert!(m.foreground.navigating());
assert_eq!(m.foreground.app(), App::Ghostty);

// after
assert_eq!(m.foregrounded, None);
```

Confirmed app after watcher report:

```rust
// before
assert_eq!(m.foreground.app(), App::Chrome);
assert!(!m.foreground.navigating());

// after
assert_eq!(
    m.foregrounded.as_ref().map(ForegroundedApp::identity),
    Some(App::Chrome)
);
// or match the enum arm when the payload matters
assert!(matches!(m.foregrounded, Some(ForegroundedApp::Chrome(_))));
```

Nav choice before report:

```rust
// before
assert_eq!(m.foreground.app(), App::Other);
assert!(m.foreground.navigating());

// after
assert_eq!(m.foregrounded, None);
```

Spotlight path (no nav, seed still Other):

```rust
// before
assert_eq!(m.foreground.app(), App::Other);
assert!(!m.foreground.navigating());

// after
assert!(matches!(m.foregrounded, Some(ForegroundedApp::Other)));
```

Direct writes in tab-URL tests:

```rust
// before
m.foreground.set_front_app(App::Chrome);

// after
m.foregrounded = Some(ForegroundedApp::from_identity(App::Chrome));
```

Every other `m.foreground.app()` / `m.foreground.navigating()` assertion follows the same pattern.

Comment on `a_pending_nav_binds_nothing_until_the_foreground_event`:

```rust
// While a nav is pending, the in-app level is empty: `foregrounded` is None, so no app's
// bindings apply in the gap. A key pressed before the foreground event lands is unbound; once
// the event lands, the chosen app's bindings apply.
```

## Docs

`docs-website/docs/interacting-with-macos/apps-and-the-frontmost-app.md`:

```rust
// before
pub struct Foreground {
    app: ForegroundedApp,
    navigating: bool,
}

// after: the field on Mercury
pub foregrounded: Option<ForegroundedApp>,
```

Prose: one copy at the root, the field itself. `record_front_app` assigns `Some(ForegroundedApp::from_identity(app))`. Nav clears to `None`. `app_data` matches `root.foregrounded`. Drop every mention of `confirmed`, `navigating`, and `struct Foreground`.

Debug dumps in `README.md` and `docs-website/docs/getting-started-with-mercury.md`:

```text
// before
Mercury { foreground: Foreground { app: Ghostty, navigating: false }, ... }

// after
Mercury { foregrounded: Some(Ghostty), ... }
```

Chrome-extension docs that print `Foreground { app: Chrome(...) }` become `foregrounded: Some(Chrome(...))`.

## End-user behavior

Unchanged:

- `n c` emits `Foreground(Chrome)`, lands in the in-app layer, binds nothing until the watcher reports Chrome, then Chrome's keys apply.
- A key in the gap is unbound.
- `i` from home still enters in-app for the already-reported front app.
- Tab URLs still drop when Chrome is not the front app (or while `foregrounded` is `None`).
- Overlay `o` on a reported in-app app still shows that app's keymap.

The one visible change: if the overlay is shown on the in-app layer while a nav is still pending, it shows the bare in-app keymap (`INAPP_OVERLAY`) rather than the previous app's keymap.

## One change

Single shippable step: delete `Foreground`, put `foregrounded: Option<ForegroundedApp>` on `Mercury`, update every call site, tests, and the docs listed above. No prefactor.
