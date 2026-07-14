//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use bind::{Bind, Node};
use freddie_keys::{Key, KeyEvent, KeyPress, PressType};
use laserbeam::{Laserbeam, Path};

use crate::handlers::*;
use crate::{AnyKey, App, Foregrounded, ForegroundEvent, MercuryEffect, MercuryEvent, MercuryStruct};

#[derive(Laserbeam, Bind, Debug)]
#[laserbeam_root(resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Foregrounded => on_foregrounded)]
pub struct Mercury {
    pub foregrounded: App,
    #[resolve_into]
    pub layer: Layer,
}

#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Resize(ResizeLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
}

#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = HomeLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer {}

#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = NavLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser.
#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = ResizeLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
)]
pub struct ResizeLayer {}

/// The typing layer: `escape` goes home, any other key passes through.
#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = TypingLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => to_home,
    AnyKey => passthru,
)]
pub struct TypingLayer {}

/// The in-app layer. It stores NO app: `root.foregrounded` is the only copy, and [`app_data`]
/// builds the app's level from it on every dispatch. There is nothing to keep in sync and
/// nothing to go stale.
#[derive(Laserbeam, Bind, Debug, Default)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
pub struct AppLayer {}

/// The app's level, which is not in the tree. Several possible levels, so the data is an enum;
/// an app with no bindings is not a variant, and [`app_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
pub enum AppData {
    Chrome(ChromeApp),
    Ghostty(GhosttyApp),
}

/// Reads `root.foregrounded`, the only copy, and builds the level for it.
///
/// A shared reference, so it cannot mutate: it derives, it does not act. `Zed` and `Other`
/// bind nothing, so they get no level and no struct.
fn app_data(path: &AppLayerPath) -> Option<AppData> {
    match path.parent().parent().foregrounded {
        App::Chrome => Some(AppData::Chrome(ChromeApp {})),
        App::Ghostty => Some(AppData::Ghostty(GhosttyApp {})),
        App::Zed | App::Other => None,
    }
}

/// Chrome's level. A unit for now: mercury tracks nothing per app. It stops being one when it
/// carries something (a tab name).
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyR.down() => refresh)]
pub struct ChromeApp {}

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

pub type LayerPath<'a> = Path<Layer, &'a mut Mercury>;
pub type HomeLayerPath<'a> = Path<HomeLayer, LayerPath<'a>>;
pub type NavLayerPath<'a> = Path<NavLayer, LayerPath<'a>>;
pub type ResizeLayerPath<'a> = Path<ResizeLayer, LayerPath<'a>>;
pub type TypingLayerPath<'a> = Path<TypingLayer, LayerPath<'a>>;
pub type AppLayerPath<'a> = Path<AppLayer, LayerPath<'a>>;
/// An app's level is not in the tree, so it is a `Node`, not a `Path`.
pub type ChromeAppNode<'a> = Node<AppLayerPath<'a>, ChromeApp>;
pub type GhosttyAppNode<'a> = Node<AppLayerPath<'a>, GhosttyApp>;

/// The active leaf the tree resolves to.
///
/// An app's level is not in the tree, so it cannot appear here: `Resolved` is an enum of
/// `Path`s and a derived level has none. Nothing calls `resolve()` anyway; see
/// `refactors/pending/resolved-is-dead-weight.md`.
pub enum Resolved<'a> {
    HomeLayer(HomeLayerPath<'a>),
    NavLayer(NavLayerPath<'a>),
    ResizeLayer(ResizeLayerPath<'a>),
    TypingLayer(TypingLayerPath<'a>),
    AppLayer(AppLayerPath<'a>),
}

impl Default for Mercury {
    fn default() -> Self {
        Self {
            foregrounded: App::Other,
            layer: Layer::Home(HomeLayer {}),
        }
    }
}

impl Mercury {
    /// Dispatches one event, returning the handler's effects, or `None` when the active state
    /// binds nothing for it.
    #[must_use]
    pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
        bind::dispatch::<MercuryStruct, Self>(self, event)
    }
}

/// A keyboard event for `key`.
#[must_use]
pub const fn key(key: Key) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent {
        key,
        press: PressType::Down,
    })
}

/// An app-foregrounded event for `app`.
#[must_use]
pub const fn foreground(app: App) -> MercuryEvent {
    MercuryEvent::Foreground(ForegroundEvent { app })
}
