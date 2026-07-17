# each layer in its own module

Every layer node moves from `state.rs` into its own module with a `new` constructor, so a layer is built only through `new` and never as a bare struct literal at a call site. Once fields are module-private, a layer whose construction has an effect (nav arming its idle-timeout) can own that in `new` and hand the effect back.

## layout

`crates/mercury/src/state.rs` becomes `crates/mercury/src/state/mod.rs`. It keeps `Mercury`, `Foreground`, `HeldModifiers`, `Side`, `LeftRightPair`, the `Layer` enum, the path aliases, and the free functions (`key`, `foreground`, `quit_event`). Each layer moves to a sibling file, re-exported so the rest of the crate names it unchanged.

`state/mod.rs` gains, near the top:

```rust
mod app;
mod home;
mod nav;
mod resize;
mod typing;

pub use app::{AppData, AppLayer, ChromeApp, GhosttyApp};
pub use home::HomeLayer;
pub use nav::NavLayer;
pub use resize::ResizeLayer;
pub use typing::TypingLayer;
```

The layer struct definitions (and `app_data`) are cut from `state.rs`; `Layer`, the path aliases (`LayerPath`, `HomeLayerPath`, …), `Mercury`, and the rest stay in `mod.rs`.

## change 1: home

New `crates/mercury/src/state/home.rs`:

```rust
use bind::Bind;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::MercuryStruct;
use super::LayerPath;

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer {}

impl HomeLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

## change 2: nav

New `crates/mercury/src/state/nav.rs`:

```rust
use bind::Bind;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::MercuryStruct;
use super::LayerPath;

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}

impl NavLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

## change 3: resize

New `crates/mercury/src/state/resize.rs`:

```rust
use bind::Bind;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::MercuryStruct;
use super::LayerPath;

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
)]
pub struct ResizeLayer {}

impl ResizeLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

## change 4: typing

New `crates/mercury/src/state/typing.rs`. `Default` is dropped: construction goes through `new`.

```rust
use bind::Bind;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::MercuryStruct;
use super::LayerPath;

/// The typing layer. It binds only `escape` (`cmd`-`escape` leaves to home); every other key
/// falls to the root, which passes it through because typing is a passthrough layer.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => maybe_go_home)]
pub struct TypingLayer {}

impl TypingLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

## change 5: app

New `crates/mercury/src/state/app.rs`, holding `AppLayer`, its derived level `AppData`, the app nodes, and `app_data`. `Default` is dropped from `AppLayer`.

```rust
use bind::{Bind, Node};
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{App, MercuryStruct};
use super::{AppLayerPath, LayerPath};

/// The in-app layer. It stores NO app: `root.foreground` is the only copy, and [`app_data`]
/// builds the app's level from it on every dispatch.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
)]
pub struct AppLayer {}

impl AppLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}

/// The app's level, which is not in the tree. Several possible levels, so the data is an enum;
/// an app with no bindings is not a variant, and [`app_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
pub enum AppData {
    Chrome(ChromeApp),
    Ghostty(GhosttyApp),
}

/// Reads the confirmed front app, the only copy, and builds the level for it.
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    let root = path.parent().parent();
    match root.foreground.confirmed() {
        Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
        Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
        _ => None,
    }
}

/// Chrome's level.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyR.down() => refresh)]
pub struct ChromeApp {}

impl ChromeApp {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}

/// Ghostty's level, where `j` and `k` walk tmux's panes.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyJ.down() => previous_window,
    Key::KeyK.down() => next_window,
    Key::Num1.down() => window_1,
    Key::Num2.down() => window_2,
    Key::Num3.down() => window_3,
    Key::Num4.down() => window_4,
    Key::Num5.down() => window_5,
    Key::Num6.down() => window_6,
    Key::Num7.down() => window_7,
    Key::Num8.down() => window_8,
    Key::Num9.down() => window_9,
    Key::Num0.down() => window_0,
)]
pub struct GhosttyApp {}

impl GhosttyApp {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

`state/mod.rs` keeps the app path aliases, now naming the re-exported types:

```rust
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;
pub type ChromeAppNode<'a> = Node<AppLayerPath<'a>, ChromeApp>;
pub type GhosttyAppNode<'a> = Node<AppLayerPath<'a>, GhosttyApp>;
```

## change 6: construct every layer through `new`

`Mercury::default`, in `state/mod.rs`, before:

```rust
            layer: Layer::Typing(TypingLayer {}),
```

after:

```rust
            layer: Layer::Typing(TypingLayer::new()),
```

The handler call sites, before → after:

- `crates/mercury/src/handlers/mod.rs`, `go_home`: `root.set_layer(HomeLayer {})` → `root.set_layer(HomeLayer::new())`.
- `crates/mercury/src/handlers/home.rs`, `to_nav`: `.set_layer(NavLayer {})` → `.set_layer(NavLayer::new())`; `to_resize`: `ResizeLayer {}` → `ResizeLayer::new()`; `to_typing`: `TypingLayer {}` → `TypingLayer::new()`; `to_inapp`: `AppLayer {}` → `AppLayer::new()`.
- `crates/mercury/src/handlers/nav.rs`, `navigate`: `set_layer(AppLayer {})` → `set_layer(AppLayer::new())`.
- `crates/mercury/src/handlers/typing.rs`, `maybe_go_home`: any `HomeLayer {}` → `HomeLayer::new()`.

`crates/mercury/tests/transitions.rs` constructs layers directly for seeding (`Mercury::with_layer(Layer::Home(HomeLayer {}))` and similar); each `XLayer {}` becomes `XLayer::new()`.
