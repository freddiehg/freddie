# The return-home wrapper

Not done.

## The problem

Four layers carry a return-home timer: nav, resize, in-app, site. Each holds its own copy of the same machinery.

```rust
pub struct NavLayer    { pub(crate) home_timeout: TimerGuard }
pub struct ResizeLayer { pub(crate) home_timeout: TimerGuard }
pub struct AppLayer    { pub(crate) home_timeout: TimerGuard }
pub struct SiteLayer   { pub(crate) home_timeout: TimerGuard }
```

The field is duplicated four times, arming it in `new()` four times, and the firing binding `|path| path.get().home_timeout.trigger() => to_home` four times. `Layer::rearm_timeout` reaches into whichever variant holds the guard. Nothing in the type says these four, and only these four, have a timer.

The timer belongs to one thing: being in a layer that returns home. Model it as a wrapper that owns the timer and wraps the group of layers that have one.

## The shape

`Layer` goes three ways. `Home` and `Typing` stay flat — grouping them under a `NotTimed` enum is a separate follow-up. The four timed layers move into `ReturnHomeLayers`, and `AndReturnHome` wraps that group and owns the one timer.

```rust
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    ReturnHome(AndReturnHome),
}

pub struct AndReturnHome {
    layers: ReturnHomeLayers,   // #[resolve_into]
    guard: TimerGuard,
}

pub enum ReturnHomeLayers {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

```
Mercury ─▶ Layer ─▶ AndReturnHome ─▶ ReturnHomeLayers ─▶ NavLayer | ResizeLayer | AppLayer | SiteLayer
```

`AndReturnHome` is a concrete node the existing derive handles — no generics. It binds the firing once and resets the timer with a co-fire bind; the four leaves lose their timer field and firing entirely; and `handle` sheds its rearm block. The one thing it needs that mercury does not have yet is co-fire dispatch itself (the Prerequisite). The shared exit keys (`escape`, `o`, `t`) stay on the leaves for now; lifting them onto the wrapper is a follow-up, as is grouping `Home`/`Typing`.

## Prerequisite

The rearm-on-activity is a co-fire bind on the wrapper, so co-fire dispatch (`co-firing-binds.md`) has to land first. That is why this is sequenced after it. The payoff: `handle` sheds the rearm entirely — no before/after discriminant check, no `rearm_after`, no `Layer::rearm_timeout` — because the co-fire does the resetting.

## The change

Atomic: the tree restructure, the leaf reparenting, the constructors, the handlers, `handle`, and the test assertions move together, because the old flat `Layer` variants disappear. Behavior is unchanged but for the extra work the co-fire rearm does on the keys that leave (a fresh timer that self-cancels; see `co-firing-binds.md`).

### The new module

New file `crates/mercury/src/state/return_home.rs`:

```rust
use bind::Bind;
use freddie::TimerGuard;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{AnyKey, MercuryEffect, MercuryStruct};

use super::{AppLayer, LayerPath, NavLayer, ResizeLayer, SiteLayer, arm_return_home};

/// The layers that return home after [`RETURN_TO_HOME_TIMEOUT`](super::RETURN_TO_HOME_TIMEOUT) of
/// idle, wrapped in the one timer they share. `AndReturnHome` owns the guard, the firing that goes
/// home, and the co-fire that resets the timer on any key reaching it; its [`layers`](Self::layers)
/// is which such layer is active. Home and typing are not here: home is the destination the timer
/// returns to, and typing is passthrough, so neither carries a timer.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
// The firing goes home (exclusive, post-descend); a key reaching the wrapper resets the timer
// (co-fire, pre-descend). Only this node's own timer: a firing from a layer already left matches
// nothing.
#[bind(|path| path.get().guard.trigger() => to_home)]
#[cofire_bind(AnyKey => rearm)]
pub struct AndReturnHome {
    #[resolve_into]
    layers: ReturnHomeLayers,
    // Read by the trigger matching its firing, and held for its `Drop`: dropping the guard cancels
    // the return-home timer.
    guard: TimerGuard,
}

impl AndReturnHome {
    /// Enter a return-home layer with its timer armed, returning the wrapper and the effect that
    /// schedules it.
    #[must_use]
    pub(crate) fn new(layers: impl Into<ReturnHomeLayers>) -> (Self, MercuryEffect) {
        let (guard, timer) = arm_return_home();
        (
            Self {
                layers: layers.into(),
                guard,
            },
            timer,
        )
    }

    /// Reset the return-home timer on in-layer activity: drop the old guard (cancelling it) and arm
    /// a fresh one, returning the effect that schedules it.
    #[must_use]
    pub(crate) fn rearm(&mut self) -> MercuryEffect {
        let (guard, timer) = arm_return_home();
        self.guard = guard;
        timer
    }

    /// `pub` because the integration test crate reads it to assert which layer is active.
    #[must_use]
    pub const fn layers(&self) -> &ReturnHomeLayers {
        &self.layers
    }
}

/// Which return-home layer is active. `derive_more::From` gives each leaf an `Into<ReturnHomeLayers>`
/// so `AndReturnHome::new(NavLayer::new())` and the like construct it.
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = AndReturnHomePath)]
#[binds(MercuryStruct)]
pub enum ReturnHomeLayers {
    Nav(NavLayer),
    Resize(ResizeLayer),
    InApp(AppLayer),
    Site(SiteLayer),
}
```

`AndReturnHome` does not derive `From` (it is built through `new`, which arms the timer). `Layer` keeps its `derive_more::From`, now yielding `From<AndReturnHome>` for `set_layer`.

### `crates/mercury/src/state/mod.rs`

Register and export the module beside the others:

```rust
mod return_home;
```
```rust
pub use return_home::{AndReturnHome, ReturnHomeLayers};
```

The `Layer` enum, before six variants; after:

```rust
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    ReturnHome(AndReturnHome),
}
```

The path aliases, before:

```rust
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;
pub type SiteLayerPath<'a> = PathMut<SiteLayer, LayerPath<'a>>;
```

after (two levels inserted; the app and site leaves hang off `ReturnHomeLayers`):

```rust
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type AndReturnHomePath<'a> = PathMut<AndReturnHome, LayerPath<'a>>;
pub type ReturnHomeLayersPath<'a> = PathMut<ReturnHomeLayers, AndReturnHomePath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, ReturnHomeLayersPath<'a>>;
pub type SiteLayerPath<'a> = PathMut<SiteLayer, ReturnHomeLayersPath<'a>>;
```

`Layer::name`, after (the four names come from the active leaf):

```rust
pub const fn name(&self) -> &'static str {
    match self {
        Self::Home(_) => "Home",
        Self::Typing(_) => "Typing",
        Self::ReturnHome(w) => match w.layers() {
            ReturnHomeLayers::Nav(_) => "Nav",
            ReturnHomeLayers::Resize(_) => "Resize",
            ReturnHomeLayers::InApp(_) => "App",
            ReturnHomeLayers::Site(_) => "Site",
        },
    }
}
```

`Layer::overlay_content`, the same reshaping (the per-leaf arms unchanged, only nested):

```rust
pub fn overlay_content(&self, foreground: &Foreground) -> &'static str {
    match self {
        Self::Home(_) => home::OVERLAY,
        Self::Typing(_) => typing::OVERLAY,
        Self::ReturnHome(w) => match w.layers() {
            ReturnHomeLayers::Nav(_) => nav::OVERLAY,
            ReturnHomeLayers::Resize(_) => resize::OVERLAY,
            ReturnHomeLayers::InApp(_) => app::overlay_for(foreground.app()),
            ReturnHomeLayers::Site(_) => site::overlay_for(
                foreground
                    .confirmed_chrome()
                    .and_then(|chrome| chrome.url.as_deref())
                    .map(Site::from_url),
            ),
        },
    }
}
```

`Layer::is_passthrough` is unchanged: `matches!(self, Self::Typing(_))`. `Layer::rearm_timeout` is deleted — the co-fire rearm on `AndReturnHome` replaces it.

`Mercury::handle` sheds the rearm block it grew for the earlier bug fix and goes back to a bare dispatch, because the co-fire does the resetting. Before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let before = std::mem::discriminant(&self.layer);
    let mut effects = bind::dispatch::<MercuryStruct, Self>(self, event)?;
    if matches!(event, MercuryEvent::Key(_))
        && std::mem::discriminant(&self.layer) == before
        && let Some(reset) = self.layer.rearm_timeout()
    {
        effects.push(reset);
    }
    Some(effects)
}
```

after (the top-level `bind::dispatch` signature is unchanged by co-fire dispatch; it builds the accumulator and returns it, per `co-firing-binds.md`):

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

The co-fire handler the bind names lives with the other handlers; it takes the co-fire `Node<&mut P, ()>` shape (`co-firing-binds.md`) and calls the wrapper's `rearm`:

```rust
pub(crate) fn rearm<'a>(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath<'a>, ()>) -> MercuryEffect {
    node.parent.get_mut().rearm()
}
```

### The four leaf layers

Each of `nav.rs`, `resize.rs`, `app.rs`, `site.rs` loses its `home_timeout` field, its firing bind, and the timer from its `new()`, and reparents to `ReturnHomeLayersPath`. Nav, before:

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

after (the escape/`o`/`t` binds stay; lifting them is a follow-up):

```rust
use super::ReturnHomeLayersPath;

#[derive(Bind, Debug)]
#[node(parent = ReturnHomeLayersPath)]
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

- `resize.rs`: keep the arrows and `r`; unit struct; `new() -> Self`; parent `ReturnHomeLayersPath`.
- `app.rs`: keep `n => to_nav`, `s => to_site`, and `#[derived_child(app_data)]`; unit struct; `new() -> Self`; parent `ReturnHomeLayersPath`. `AppData`/`ChromeApp`/`GhosttyApp` are unchanged: their `#[derived_node(parent = AppLayerPath)]` follows the reparented alias.
- `site.rs`: keep `escape`/`o`/`t` and `#[derived_child(site_data)]`; unit struct; `new() -> Self`; parent `ReturnHomeLayersPath`. `SiteData`/`ClaudeAiSite` are unchanged.

Each leaf drops `use freddie::TimerGuard;` and its `arm_return_home` import, and imports `ReturnHomeLayersPath` in place of `LayerPath`.

### The handlers

Every handler that entered a return-home layer built the leaf and pushed the leaf's timer; now it wraps the leaf in `AndReturnHome::new`, which arms the timer. `home.rs`, `to_nav` before:

```rust
let (nav, timer) = NavLayer::new();
let mut effects = node.parent.ascend_mut().set_layer(nav);
effects.push(timer);
effects
```

after:

```rust
let (wrapped, timer) = AndReturnHome::new(NavLayer::new());
let mut effects = node.parent.ascend_mut().set_layer(wrapped);
effects.push(timer);
effects
```

The same edit applies to `to_inapp`, `to_site`, and `to_resize` in `home.rs`, and to `navigate` in `nav.rs`:

```rust
let (wrapped, timer) = AndReturnHome::new(AppLayer::new());
let mut effects = root.set_layer(wrapped);
effects.push(timer);
effects.push(MercuryEffect::Foreground(app));
effects
```

`set_layer(impl Into<Layer>)` is unchanged: `From<AndReturnHome> for Layer` comes from `Layer`'s derive, and `AndReturnHome::new` takes `impl Into<ReturnHomeLayers>` so each leaf's `From` feeds it. `to_typing` and `go_home` (typing and home are not wrapped) are unchanged. The handler modules add `AndReturnHome` to their `use crate::state::{..}` imports.

### `crates/mercury/src/lib.rs`

Add the two types to the state re-export beside the other layers:

```rust
pub use state::{
    .., AndReturnHome, ReturnHomeLayers, ..
};
```

### The tests

`crates/mercury/tests/transitions.rs` inspects the active layer about forty times, matching the flat enum. The four return-home layers now sit under `Layer::ReturnHome`, so those assertions match the nested structure through a helper that reaches the kind:

```rust
// The active return-home layer, or `None` when the active layer is home or typing.
fn return_home(m: &Mercury) -> Option<&ReturnHomeLayers> {
    match m.layer() {
        Layer::ReturnHome(w) => Some(w.layers()),
        _ => None,
    }
}
```

Each `matches!(m.layer(), Layer::X(_))` for a return-home layer becomes the nested match, one for one (`Nav`, `Resize`, `InApp`, `Site` keep their names), keeping any trailing message:

```rust
assert!(matches!(return_home(&m), Some(ReturnHomeLayers::Nav(_))));
assert!(matches!(return_home(&m), Some(ReturnHomeLayers::InApp(_))), "{app:?} left nav");
```

`Layer::Home(_)` and `Layer::Typing(_)` are unchanged, including the inline-temporary form in `default_boots_into_typing`, and the `home()` helper's `Mercury::with_layer(Layer::Home(HomeLayer))` construction. The test file adds `ReturnHomeLayers` to its imports.

## Follow-ups

Two changes this doc deliberately leaves out, each its own later doc:

- Lift the shared exits (`escape`, `o`, `t`) from the four leaves onto `AndReturnHome`, since they are identical across all four and the wrapper is on the active path below each.
- Group `Home` and `Typing` under a `NotTimed` enum, so `Layer` becomes the clean `ReturnHome | NotTimed` split.

## Note

`AndReturnHome` is a concrete named node with a `#[resolve_into]` field, which the existing derive handles — no generics, no new node machinery. Its one dependency is co-fire dispatch (`co-firing-binds.md`), for the `#[cofire_bind]` rearm; that is the Prerequisite and the reason for the sequencing. The positional-`#[resolve_into]` capability (`positional-resolve-into.md`, landed) is not exercised by this shape — a two-field wrapper is a named struct, not a tuple.
