//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use bind::{Bind, Node};
use freddie_keys::{Key, KeyEvent, PressType};
use laserbeam::PathMut;

use crate::handlers::*;
use crate::{AnyKey, App, Foregrounded, ForegroundEvent, MercuryEffect, MercuryEvent, MercuryStruct};

#[derive(Bind, Debug)]
#[laserbeam_root]
#[binds(MercuryStruct)]
#[bind(Foregrounded => on_foregrounded)]
pub struct Mercury {
    pub foregrounded: App,
    #[resolve_into]
    pub layer: Layer,
}

#[derive(Bind, Debug)]
#[laserbeam(path = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Resize(ResizeLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
}

#[derive(Bind, Debug)]
#[laserbeam(path = HomeLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer {}

#[derive(Bind, Debug)]
#[laserbeam(path = NavLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser.
#[derive(Bind, Debug)]
#[laserbeam(path = ResizeLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
)]
pub struct ResizeLayer {}

/// The keys typing is tracking as held. Just `cmd` for now; extend it with more fields, or
/// switch to a `HashSet<Key>`, as more held-key conditions are needed. It is tracked here
/// rather than at the root because dispatch fires one handler per event and typing's own
/// catch-all is the handler that sees each key. The known gap: a modifier pressed in typing
/// and released after leaving typing is not un-tracked here, so its emitted down is not
/// closed by an emitted up.
#[derive(Debug, Default)]
pub struct SetOfHeldKeys {
    pub cmd: bool,
}

/// The typing layer: any key passes through, tracking which of the watched keys are held.
/// `escape` passes through too, unless `cmd` is held, in which case it exits to home.
#[derive(Bind, Debug, Default)]
#[laserbeam(path = TypingLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyKey => modify_held_and_pass_through,
)]
pub struct TypingLayer {
    pub held: SetOfHeldKeys,
}

/// The in-app layer. It stores NO app: `root.foregrounded` is the only copy, and [`app_data`]
/// builds the app's level from it on every dispatch. There is nothing to keep in sync and
/// nothing to go stale.
#[derive(Bind, Debug, Default)]
#[laserbeam(path = AppLayerPath)]
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

pub type LayerPath<'a> = PathMut<Layer, &'a mut Mercury>;
pub type HomeLayerPath<'a> = PathMut<HomeLayer, LayerPath<'a>>;
pub type NavLayerPath<'a> = PathMut<NavLayer, LayerPath<'a>>;
pub type ResizeLayerPath<'a> = PathMut<ResizeLayer, LayerPath<'a>>;
pub type TypingLayerPath<'a> = PathMut<TypingLayer, LayerPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;
/// An app's level is not in the tree, so it is a `Node`, not a `PathMut`.
pub type ChromeAppNode<'a> = Node<AppLayerPath<'a>, ChromeApp>;
pub type GhosttyAppNode<'a> = Node<AppLayerPath<'a>, GhosttyApp>;

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
