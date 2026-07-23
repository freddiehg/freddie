# The timed-layer wrapper

Not done.

## The problem

Four layers carry a return-home timer: nav, resize, in-app, site. Each holds its own copy of the same machinery.

```rust
pub struct NavLayer    { pub(crate) home_timeout: TimerGuard }
pub struct ResizeLayer { pub(crate) home_timeout: TimerGuard }
pub struct AppLayer    { pub(crate) home_timeout: TimerGuard }
pub struct SiteLayer   { pub(crate) home_timeout: TimerGuard }
```

The field is duplicated four times. Arming it in `new()` is duplicated four times. The firing binding `|path| path.get().home_timeout.trigger() => to_home` is duplicated four times. `Layer::rearm_timeout` is a four-arm match that reaches into whichever variant holds the guard. Nothing in the type system says these four, and only these four, have a timer; `rearm_timeout` has to spell out that home and typing do not.

The four also share three keys, again copied into each: `escape => to_home`, `o => toggle_overlay`, `t => to_typing`.

The timer, and the shared exits, belong to ONE thing: being in a layer that returns home. Model that thing.

## The shape

`Layer` splits three ways. Home and typing stay as they are: home is the destination the timer returns to, and typing is the passthrough (login-safe) layer, so neither carries a timer. The other four move under a wrapper that owns the one timer and the exits shared by all of them.

```rust
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    Timed(TimedLayer),
}

pub struct TimedLayer {
    home_timeout: TimerGuard,
    kind: TimedKind,          // #[resolve_into]
}

pub enum TimedKind {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

The tree gains two levels. `TimedLayer` is a place under `Layer`; it resolves into `TimedKind`; each `TimedKind` variant is one of the four leaf layers, which now carry no timer.

```
Mercury ─▶ Layer ─▶ TimedLayer ─▶ TimedKind ─▶ NavLayer | ResizeLayer | AppLayer | SiteLayer
```

`TimedLayer` binds the firing and the three shared exits, once. Each leaf keeps only its own keys (nav's app-openers, resize's arrows, in-app's `n`/`s`, site's derived child). `rearm_timeout` collapses to one arm. The four `home_timeout` fields, four `new()` arms, four firing binds, and three-times-four shared exits become one of each.

## Change 1: introduce the wrapper, moving the timer

The consequential change. It is atomic: the tree restructure, the leaf reparenting, the constructors, the handlers, `handle`'s activity check, and the test assertions all move together, because the old flat `Layer` variants disappear. The three shared exit keys stay on the leaves for now (Change 2 lifts them), so this change moves only the timer and the structure.

Behavior is unchanged.

### The new module

New file `crates/mercury/src/state/timed.rs`:

```rust
use bind::Bind;
use freddie::TimerGuard;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{MercuryEffect, MercuryStruct};

use super::{AppLayer, LayerPath, NavLayer, ResizeLayer, SiteLayer, TimedLayerPath, arm_return_home};

/// A layer that returns home after [`RETURN_TO_HOME_TIMEOUT`](super::RETURN_TO_HOME_TIMEOUT) of
/// idle. It owns the one return-home timer; its [`kind`](Self::kind) is which such layer is active.
/// Home and typing are not here: home is the destination the timer returns to, and typing is the
/// passthrough layer, so neither carries a timer.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    // Only this timer: a firing from a timed layer already left matches nothing.
    |path| path.get().home_timeout.trigger() => to_home,
)]
pub struct TimedLayer {
    // Read by the trigger matching its firing, and held for its `Drop`: dropping the guard cancels
    // the return-home timer.
    home_timeout: TimerGuard,
    #[resolve_into]
    kind: TimedKind,
}

impl TimedLayer {
    /// Enter a timed layer with its return-home timer armed, returning the layer and the effect
    /// that schedules it.
    #[must_use]
    pub(crate) fn new(kind: impl Into<TimedKind>) -> (Self, MercuryEffect) {
        let (home_timeout, timer) = arm_return_home();
        (
            Self {
                home_timeout,
                kind: kind.into(),
            },
            timer,
        )
    }

    /// Reset the return-home timer on in-layer activity: drop the old guard (cancelling it) and arm
    /// a fresh one, returning the effect that schedules it.
    #[must_use]
    pub(crate) fn rearm(&mut self) -> MercuryEffect {
        let (guard, timer) = arm_return_home();
        self.home_timeout = guard;
        timer
    }

    /// `pub` (not `pub(crate)`) because the integration test crate reads it to assert which timed
    /// layer is active.
    #[must_use]
    pub const fn kind(&self) -> &TimedKind {
        &self.kind
    }
}

/// Which timed layer is active. `derive_more::From` gives each leaf an `Into<TimedKind>`, so
/// `TimedLayer::new(NavLayer::new())` and the like construct it.
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = TimedLayerPath)]
#[binds(MercuryStruct)]
pub enum TimedKind {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

`TimedLayer` does not derive `From`: it is built through `new`, which arms the timer. `TimedKind` derives `From` so a leaf converts into it. `Layer` keeps its own `derive_more::From`, which now yields `From<TimedLayer>` (used by `set_layer`).

### `crates/mercury/src/state/mod.rs`

Register the module, beside the others:

```rust
mod app;
mod home;
mod nav;
mod resize;
mod site;
mod timed;
mod typing;
```

Export the two new types, beside the others:

```rust
pub use timed::{TimedKind, TimedLayer};
```

The `Layer` enum, before:

```rust
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Resize(ResizeLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

after:

```rust
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    Timed(TimedLayer),
}
```

The path aliases, before:

```rust
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;
pub type SiteLayerPath<'a> = PathMut<SiteLayer, LayerPath<'a>>;
```

after (two levels inserted; the app and site leaves now hang off `TimedKind`):

```rust
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type TimedLayerPath<'a> = PathMut<TimedLayer, LayerPath<'a>>;
pub type TimedKindPath<'a> = PathMut<TimedKind, TimedLayerPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, TimedKindPath<'a>>;
pub type SiteLayerPath<'a> = PathMut<SiteLayer, TimedKindPath<'a>>;
```

`Layer::name`, before matches all six; after, the four timed names come from the kind:

```rust
pub const fn name(&self) -> &'static str {
    match self {
        Self::Home(_) => "Home",
        Self::Typing(_) => "Typing",
        Self::Timed(t) => match t.kind() {
            TimedKind::Nav(_) => "Nav",
            TimedKind::Resize(_) => "Resize",
            TimedKind::InApp(_) => "App",
            TimedKind::Site(_) => "Site",
        },
    }
}
```

`Layer::overlay_content`, same reshaping (the per-kind arms are unchanged, only nested):

```rust
pub fn overlay_content(&self, foreground: &Foreground) -> &'static str {
    match self {
        Self::Home(_) => home::OVERLAY,
        Self::Typing(_) => typing::OVERLAY,
        Self::Timed(t) => match t.kind() {
            TimedKind::Nav(_) => nav::OVERLAY,
            TimedKind::Resize(_) => resize::OVERLAY,
            TimedKind::InApp(_) => app::overlay_for(foreground.app()),
            TimedKind::Site(_) => site::overlay_for(
                foreground
                    .confirmed_chrome()
                    .and_then(|chrome| chrome.url.as_deref())
                    .map(Site::from_url),
            ),
        },
    }
}
```

`Layer::is_passthrough` is unchanged: `matches!(self, Self::Typing(_))`.

`Layer::rearm_timeout`, before (the four-arm reach-in):

```rust
fn rearm_timeout(&mut self) -> Option<MercuryEffect> {
    let home_timeout = match self {
        Self::Nav(nav) => &mut nav.home_timeout,
        Self::Resize(resize) => &mut resize.home_timeout,
        Self::InApp(inapp) => &mut inapp.home_timeout,
        Self::Site(site) => &mut site.home_timeout,
        Self::Home(_) | Self::Typing(_) => return None,
    };
    let (guard, timer) = arm_return_home();
    *home_timeout = guard;
    Some(timer)
}
```

after (one arm; the wrapper owns the arming):

```rust
fn rearm_timeout(&mut self) -> Option<MercuryEffect> {
    match self {
        Self::Timed(timed) => Some(timed.rearm()),
        Self::Home(_) | Self::Typing(_) => None,
    }
}
```

### The activity check in `handle`

`handle` rearms when a keypress leaves you in the same layer. It compares the layer's discriminant before and after dispatch. With the flat enum that discriminant distinguished all six layers; with the wrapper it distinguishes only three, so two different timed layers now share one discriminant. A keypress that moves between timed layers (in-app `n` into nav, nav `c` into in-app) already arms a fresh timer in its handler, so the discriminant-only check would rearm a second time and strand the first timer.

Compare a token that includes the timed kind, so a move between two timed layers reads as a change. Add to `impl Layer`:

```rust
/// Equal across a keypress exactly when the same layer is still active, timed kinds included.
/// `handle` rearms only when this holds: a transition changes it (a move between two timed layers
/// changes the kind), and the transition already armed its own timer, so the rearm never doubles.
fn activity_token(&self) -> (std::mem::Discriminant<Self>, Option<std::mem::Discriminant<TimedKind>>) {
    (
        std::mem::discriminant(self),
        match self {
            Self::Timed(t) => Some(std::mem::discriminant(t.kind())),
            _ => None,
        },
    )
}
```

`Mercury::handle`, before:

```rust
let before = std::mem::discriminant(&self.layer);
let mut effects = bind::dispatch::<MercuryStruct, Self>(self, event)?;
// A keypress that leaves you in the same layer is activity: reset that layer's return-home
// timer, so it fires only after you go idle, not a fixed span after you entered.
if matches!(event, MercuryEvent::Key(_))
    && std::mem::discriminant(&self.layer) == before
    && let Some(reset) = self.layer.rearm_timeout()
{
    effects.push(reset);
}
Some(effects)
```

after:

```rust
let before = self.layer.activity_token();
let mut effects = bind::dispatch::<MercuryStruct, Self>(self, event)?;
// A keypress that leaves you in the same layer is activity: reset that layer's return-home
// timer, so it fires only after you go idle, not a fixed span after you entered.
if matches!(event, MercuryEvent::Key(_))
    && self.layer.activity_token() == before
    && let Some(reset) = self.layer.rearm_timeout()
{
    effects.push(reset);
}
Some(effects)
```

### The four leaf layers

Each of `nav.rs`, `resize.rs`, `app.rs`, `site.rs` loses its `home_timeout` field, its firing bind, and the timer from its `new()`, and reparents to `TimedKindPath`. Nav, before:

```rust
use freddie::TimerGuard;
use super::{LayerPath, arm_return_home};

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => to_typing,
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
    Key::Space.down() => open_spotlight,
)]
pub struct NavLayer {
    pub(crate) home_timeout: TimerGuard,
}

impl NavLayer {
    #[must_use]
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home();
        (Self { home_timeout: timeout }, timer)
    }
}
```

Nav, after (the escape/`o`/`t` binds stay for now; Change 2 lifts them):

```rust
use super::TimedKindPath;

#[derive(Bind, Debug)]
#[node(parent = TimedKindPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => to_typing,
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
    Key::Space.down() => open_spotlight,
)]
pub struct NavLayer;

impl NavLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
```

The other three take the same treatment, keeping their own binds:

- `resize.rs`: keep the arrows and `r`; unit struct; `new() -> Self`; parent `TimedKindPath`.
- `app.rs`: keep `n => to_nav` and `s => to_site`; keep `#[derived_child(app_data)]`; unit struct; `new() -> Self`; parent `TimedKindPath`. `AppData`/`ChromeApp`/`GhosttyApp` are unchanged: their `#[derived_node(parent = AppLayerPath)]` follows the reparented `AppLayerPath` alias.
- `site.rs`: it bound only the shared exits, so until Change 2 lifts them it keeps `escape`/`o`/`t` and its `#[derived_child(site_data)]`; unit struct; `new() -> Self`; parent `TimedKindPath`. `SiteData`/`ClaudeAiSite` are unchanged.

Each leaf drops `use freddie::TimerGuard;` and its `arm_return_home` import, and imports `TimedKindPath` in place of `LayerPath`.

### The handlers

Every handler that entered a timed layer built the leaf and pushed the leaf's timer. Now it wraps the leaf in `TimedLayer::new`, which arms the timer. `home.rs`, `to_nav` before:

```rust
let (nav, timer) = NavLayer::new();
let mut effects = node.parent.ascend_mut().set_layer(nav);
effects.push(timer);
effects
```

after:

```rust
let (timed, timer) = TimedLayer::new(NavLayer::new());
let mut effects = node.parent.ascend_mut().set_layer(timed);
effects.push(timer);
effects
```

The same edit applies to `to_inapp`, `to_site`, and `to_resize` in `home.rs`, and to `navigate` in `nav.rs`:

```rust
let (timed, timer) = TimedLayer::new(AppLayer::new());
let mut effects = root.set_layer(timed);
effects.push(timer);
effects.push(MercuryEffect::Foreground(app));
effects
```

`set_layer(impl Into<Layer>)` is unchanged: `From<TimedLayer> for Layer` comes from `Layer`'s derive, and `TimedLayer::new` takes `impl Into<TimedKind>` so each leaf's `From` feeds it. `to_typing` (typing is not timed) and `go_home` (home is not timed) are unchanged.

The handler modules add `TimedLayer` to their `use crate::state::{..}` imports.

### `crates/mercury/src/lib.rs`

Add the two types to the state re-export, beside the other layers:

```rust
pub use state::{
    .., TimedKind, TimedLayer, ..
};
```

### The tests

`crates/mercury/tests/transitions.rs` inspects the active layer about forty times, matching the flat enum:

```rust
assert!(matches!(m.layer(), Layer::Nav(_)));
assert!(matches!(m.layer(), Layer::InApp(_)), "{app:?} left nav");
```

The four timed layers now sit under `Layer::Timed`, so those assertions match the nested structure through a helper that reaches the kind:

```rust
// The active timed layer's kind, or `None` when the active layer is home or typing. Lets a test
// assert which timed layer it landed in against the real structure.
fn timed_kind(m: &Mercury) -> Option<&TimedKind> {
    match m.layer() {
        Layer::Timed(t) => Some(t.kind()),
        _ => None,
    }
}
```

Each `matches!(m.layer(), Layer::X(_))` for a timed layer becomes the nested match, one for one (`Nav`, `Resize`, `InApp`, `Site` keep their names), and keeps any trailing message:

```rust
assert!(matches!(timed_kind(&m), Some(TimedKind::Nav(_))));
assert!(matches!(timed_kind(&m), Some(TimedKind::InApp(_))), "{app:?} left nav");
```

Home and typing keep their direct match, since they are still `Layer` variants: `matches!(m.layer(), Layer::Home(_))` and `matches!(m.layer(), Layer::Typing(_))` are unchanged, including the inline-temporary form in `default_boots_into_typing`. The `home()` helper's `Mercury::with_layer(Layer::Home(HomeLayer))` construction is unchanged. The test file adds `TimedKind` to its imports.

## Change 2: lift the shared exits onto the wrapper

Independently shippable after Change 1, and behavior-preserving. The three keys common to all four timed layers move from each leaf up to `TimedLayer`, which is on the active path below every one of them.

`escape => to_home`, `o => toggle_overlay`, `t => to_typing` are identical across nav, resize, in-app, and site. Dispatch tries the leaf before the wrapper, so lifting changes nothing: no leaf binds these to anything else, and none of the derived app or site levels bind escape, `o`, or `t`. Home keeps its own copies (it is not under the wrapper).

`TimedLayer`'s `#[bind]`, after:

```rust
#[bind(
    // Only this timer: a firing from a timed layer already left matches nothing.
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyT.down() => to_typing,
)]
```

This needs `use freddie_keys::Key;` in `timed.rs`.

Each leaf drops those three lines. Nav keeps only its openers:

```rust
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
    Key::Space.down() => open_spotlight,
)]
pub struct NavLayer;
```

Resize keeps its arrows and `r`. In-app keeps `n => to_nav` and `s => to_site`. Site bound only the three shared exits, so after they lift it binds nothing and is left with just its derived child:

```rust
#[derive(Bind, Debug)]
#[node(parent = TimedKindPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
pub struct SiteLayer;
```

## What it buys

The return-home timer is one field, armed in one place, fired by one binding, reset by one method. The four layers that have it are exactly the variants of `TimedKind`; home and typing cannot accidentally grow one, and `rearm_timeout` no longer enumerates who does and does not. The three shared exits are written once. A fifth timed layer is a new `TimedKind` variant and its own keys, nothing more.
