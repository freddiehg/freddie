# `Foreground.app` is `Option`; drop `navigating`

A nav choice clears the front app until the watcher reports the one that actually came up. That gap is `app: None`, not a parallel bool next to a stale previous app.

## Shape

Before:

```rust
/// The frontmost app, and whether a navigation is in flight.
///
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
/// The confirmed frontmost app.
///
/// `None` while a nav choice has foregrounded an app the watcher has not yet reported: the in-app
/// level binds nothing until it does (see [`app_data`]). The field is private; the handlers drive
/// it through the methods below.
#[derive(Debug)]
pub struct Foreground {
    app: Option<ForegroundedApp>,
}
```

`None` is the only unconfirmed state. There is no previous-app value to read in the gap, and no second field that must stay consistent with the first.

## Methods

Before:

```rust
impl Foreground {
    #[must_use]
    pub const fn new(app: App) -> Self {
        Self {
            app: ForegroundedApp::from_identity(app),
            navigating: false,
        }
    }

    pub const fn start_navigating(&mut self) {
        self.navigating = true;
    }

    pub fn set_front_app(&mut self, app: App) {
        self.app = ForegroundedApp::from_identity(app);
        self.navigating = false;
    }

    pub fn set_tab_url(&mut self, url: String) {
        if self.navigating {
            return;
        }
        if let ForegroundedApp::Chrome(chrome) = &mut self.app {
            chrome.url = Some(url);
        }
    }

    #[must_use]
    pub const fn confirmed_chrome(&self) -> Option<&ForegroundedChrome> {
        match (&self.app, self.navigating) {
            (ForegroundedApp::Chrome(chrome), false) => Some(chrome),
            _ => None,
        }
    }

    #[must_use]
    pub const fn confirmed(&self) -> Option<App> {
        if self.navigating {
            None
        } else {
            Some(self.app.identity())
        }
    }

    #[must_use]
    pub const fn app(&self) -> App {
        self.app.identity()
    }

    #[must_use]
    pub const fn navigating(&self) -> bool {
        self.navigating
    }
}
```

After:

```rust
impl Foreground {
    /// The frontmost app at boot.
    ///
    /// No `Default`: a `Foreground` that does not know which app is frontmost would answer
    /// `None`, and the in-app layer would resolve against no app until the first watcher report.
    /// Construction always seeds from the OS (see `seed-at-construction`).
    #[must_use]
    pub const fn new(app: App) -> Self {
        Self {
            app: Some(ForegroundedApp::from_identity(app)),
        }
    }

    /// A nav choice foregrounded an app; the watcher has not confirmed it, so the front app is
    /// unknown until it does. From the nav handlers, and undone by [`set_front_app`](Self::set_front_app).
    pub const fn start_navigating(&mut self) {
        self.app = None;
    }

    /// The watcher reported the front app: record it. From
    /// [`record_front_app`](crate::handlers).
    pub fn set_front_app(&mut self, app: App) {
        self.app = Some(ForegroundedApp::from_identity(app));
    }

    /// The tab source reported the front tab's URL. Kept only while Chrome is the confirmed front
    /// app: a URL arriving while anything else is up, or while a nav is in flight, describes a
    /// window nobody is looking at.
    pub fn set_tab_url(&mut self, url: String) {
        if let Some(ForegroundedApp::Chrome(chrome)) = &mut self.app {
            chrome.url = Some(url);
        }
    }

    /// The confirmed front Chrome, or `None` whenever anything else is up or a nav is in flight.
    #[must_use]
    pub const fn confirmed_chrome(&self) -> Option<&ForegroundedChrome> {
        match &self.app {
            Some(ForegroundedApp::Chrome(chrome)) => Some(chrome),
            _ => None,
        }
    }

    /// The confirmed front app, or `None` while a navigation is in flight, so a key pressed in the
    /// gap does not reach the old app's bindings.
    #[must_use]
    pub const fn app(&self) -> Option<App> {
        match &self.app {
            Some(app) => Some(app.identity()),
            None => None,
        }
    }
}
```

`confirmed` and `navigating` go away. Call sites that meant the confirmed identity use `app()`; a pending nav is `app().is_none()`.

`start_navigating` and `set_front_app` keep their names and call sites (`handlers/nav.rs`, `handlers/foreground.rs`).

## Call sites of the removed methods

### `confirmed()` → `app()`

`crates/mercury/src/state/app.rs`, `app_data`:

```rust
// before
match root.foreground.confirmed() {
    Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
    Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
    _ => None,
}

// after
match root.foreground.app() {
    Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
    Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
    _ => None,
}
```

Doc comment on `app_data` already says `None` while a nav is in flight; it still does, via `app()`.

### Overlay while in-app and unconfirmed

`Layer::overlay_content` today reads the stale previous app via `foreground.app()` (always `App`). After the change that value is gone; mid-nav the overlay shows the bare in-app keymap.

```rust
// before
Self::InApp(_) => app::overlay_for(foreground.app()),

// after
Self::InApp(_) => match foreground.app() {
    Some(app) => app::overlay_for(app),
    None => app::INAPP_OVERLAY,
},
```

`INAPP_OVERLAY` is already the fallback for Finder / Zed / Other. A pending nav is the same situation: the layer is up, no app-specific keymap is confirmed.

The site arm is unchanged: it already goes through `confirmed_chrome()`.

### `confirmed_chrome` and `set_tab_url` callers

Unchanged signatures from the outside. Bodies are as above. Call sites stay:

- `handlers/tab.rs` → `set_tab_url`
- `handlers/app.rs` → `confirmed_chrome` (copy URL)
- `state/site.rs` → `confirmed_chrome` (`site_data`)
- `state/mod.rs` site overlay arm → `confirmed_chrome`

### Tests (`crates/mercury/tests/transitions.rs`)

Every `m.foreground.app()` comparison becomes `Option`. Every `navigating()` check becomes `app().is_none()` / `app().is_some()`.

Pending-nav gap (today asserts old app still present):

```rust
// before
assert!(m.foreground.navigating());
assert_eq!(m.foreground.app(), App::Ghostty);

// after
assert_eq!(m.foreground.app(), None);
```

Confirmed app after watcher report:

```rust
// before
assert_eq!(m.foreground.app(), App::Chrome);
assert!(!m.foreground.navigating());

// after
assert_eq!(m.foreground.app(), Some(App::Chrome));
```

Nav choice before report (`nav_c_foregrounds_chrome_and_enters_inapp` and `every_nav_choice_enters_inapp`):

```rust
// before
assert_eq!(m.foreground.app(), App::Other);
assert!(m.foreground.navigating());

// after
assert_eq!(m.foreground.app(), None);
```

Spotlight path (no nav pending, seed app still Other):

```rust
// before
assert_eq!(m.foreground.app(), App::Other);
assert!(!m.foreground.navigating());

// after
assert_eq!(m.foreground.app(), Some(App::Other));
```

`matches!(m.foreground.app(), App::Zed | App::Other)` becomes `matches!(m.foreground.app(), Some(App::Zed | App::Other))`.

Direct `set_front_app` in tab-URL tests keeps working; assertions use `Some(...)`.

The comment on `a_pending_nav_binds_nothing_until_the_foreground_event` updates with the shape:

```rust
// While a nav is pending, the in-app level is empty: `foreground.app()` is None, so no app's
// bindings apply in the gap. A key pressed before the foreground event lands is unbound; once
// the event lands, the chosen app's bindings apply.
```

## Docs that print the state

`docs-website/docs/interacting-with-macos/apps-and-the-frontmost-app.md`:

```rust
// before
pub struct Foreground {
    app: ForegroundedApp,
    navigating: bool,
}

// after
pub struct Foreground {
    app: Option<ForegroundedApp>,
}
```

Prose next to it: `record_front_app` calls `set_front_app`, which records the app. `app_data` matches `root.foreground.app()` (not `confirmed()`). Pending nav is `app == None`; a nav choice clears the field, and the watcher fills it.

`app_data` snippet in that page:

```rust
// before
match root.foreground.confirmed() {

// after
match root.foreground.app() {
```

And the paragraph that says `confirmed` returns `None` while navigating, and that `app` is still the previous one: rewrite so the gap is `app: None` and there is no previous-app value in the model.

Debug dumps in:

- `README.md`
- `docs-website/docs/getting-started-with-mercury.md`

```text
// before
Mercury { foreground: Foreground { app: Ghostty, navigating: false }, ... }

// after
Mercury { foreground: Foreground { app: Some(Ghostty) }, ... }
```

(Exact `Debug` of the `ForegroundedApp` arm is whatever the enum prints; the dump only needs to show `Some(...)` and no `navigating`.)

Chrome-extension docs that print `Foreground { app: Chrome(...) }` without `navigating` already match once `app` is wrapped in `Some`.

## End-user behavior

Unchanged:

- `n c` emits `Foreground(Chrome)`, lands in the in-app layer, binds nothing until the watcher reports Chrome, then Chrome's keys apply.
- A key in the gap is unbound.
- `i` from home still enters in-app for the already-confirmed front app.
- Tab URLs still drop when Chrome is not confirmed.
- Overlay `o` on a confirmed in-app app still shows that app's keymap.

The one visible change: if the overlay is shown on the in-app layer while a nav is still pending, it shows the bare in-app keymap (`INAPP_OVERLAY`) rather than the previous app's keymap.

## One change

Single shippable step: the type, the methods, every call site, the tests, the docs listed above. No prefactor.
