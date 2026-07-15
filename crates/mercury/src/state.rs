//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use bind::{Bind, Node};
use freddie_keys::{Key, KeyEvent, PressType};
use laserbeam::PathMut;

// The derive generates a call to each named handler at its node's definition site below, so
// every handler has to be in scope here. A glob keeps this in step with the handler set instead
// of a name-by-name list that drifts.
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{
    AnyKey, App, Foregrounded, ForegroundEvent, MercuryEffect, MercuryEvent, MercuryStruct, Quit,
    QuitEvent,
};

#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
)]
pub struct Mercury {
    pub foregrounded: App,
    /// Set when a nav choice foregrounds an app, cleared when the watcher reports the
    /// front app. True means a navigation is in flight: `foregrounded` is still the
    /// old app until the foreground event lands, so the in-app level stays empty
    /// rather than binding the old app in the gap. See [`app_data`].
    pub has_navigated: bool,
    #[resolve_into]
    pub layer: Layer,
}

#[derive(Bind, Debug)]
#[node(parent = MercuryPath)]
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
#[node(parent = LayerPath)]
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
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
)]
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
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    let root = path.parent().parent();
    // A navigation is in flight: the foreground event has not landed, so
    // `foregrounded` is still the previous app. Bind nothing until the watcher
    // confirms the new front app and clears the flag, so a key pressed in the gap
    // does not reach the old app's bindings.
    if root.has_navigated {
        return None;
    }
    match root.foregrounded {
        App::Chrome => Some(AppData::Chrome(ChromeApp {})),
        App::Ghostty => Some(AppData::Ghostty(GhosttyApp {})),
        App::Finder | App::Zed | App::Other => None,
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

/// The root's path is `&mut Self`; naming it lets the root's children say `parent = MercuryPath`.
pub type MercuryPath<'a> = &'a mut Mercury;
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
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
            has_navigated: false,
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

/// A quit-request event (the menu bar's Quit).
#[must_use]
pub const fn quit_event() -> MercuryEvent {
    MercuryEvent::Quit(QuitEvent)
}
