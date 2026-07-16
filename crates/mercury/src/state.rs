//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use bind::{Bind, Node};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};
use laserbeam::PathMut;

// The derive generates a call to each named handler at its node's definition site below, so
// every handler has to be in scope here. A glob keeps this in step with the handler set instead
// of a name-by-name list that drifts.
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::effect::emit;
use crate::{
    AnyModifierKey, AnyNonModifierKey, App, Foregrounded, ForegroundEvent, MercuryEffect,
    MercuryEvent, MercuryStruct, Quit, QuitEvent,
};

#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
pub struct Mercury {
    pub foregrounded: App,
    /// Set when a nav choice foregrounds an app, cleared when the watcher reports the
    /// front app. True means a navigation is in flight: `foregrounded` is still the
    /// old app until the foreground event lands, so the in-app level stays empty
    /// rather than binding the old app in the gap. See [`app_data`].
    pub has_navigated: bool,
    /// The physical truth about which modifier keys are down, updated by [`on_modifier`] on every
    /// modifier event in every layer. It has to outlive the layer, because entering and leaving a
    /// passthrough layer reads it to synchronize the app's modifier view. See [`HeldModifiers`].
    pub held: HeldModifiers,
    /// The active layer. Private, and written only through [`set_layer`](Mercury::set_layer), so
    /// no transition can change the layer without going through the modifier flush.
    #[resolve_into]
    layer: Layer,
}

#[derive(Bind, Debug, derive_more::From)]
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

impl Layer {
    /// A passthrough layer re-emits every key the active layer did not bind. Typing is the only
    /// one; add more by returning true for them.
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        matches!(self, Self::Typing(_))
    }
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

/// The typing layer. It binds only `escape` (`cmd`-`escape` leaves to home); every other key
/// falls to the root, which passes it through because typing is a passthrough layer.
#[derive(Bind, Debug, Default)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => maybe_go_home)]
pub struct TypingLayer {}

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
    // AppLayer -> Layer -> Mercury.
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
            held: HeldModifiers::default(),
            layer: Layer::Home(HomeLayer {}),
        }
    }
}

impl Mercury {
    /// A fresh Mercury with `layer` active. For construction (tests, seeding); a live transition
    /// goes through [`set_layer`](Self::set_layer).
    #[must_use]
    pub fn with_layer(layer: Layer) -> Self {
        Self {
            layer,
            ..Self::default()
        }
    }

    /// Dispatches one event, returning the handler's effects, or `None` when the active state
    /// binds nothing for it.
    #[must_use]
    pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
        bind::dispatch::<MercuryStruct, Self>(self, event)
    }

    /// The active layer.
    #[must_use]
    pub const fn layer(&self) -> &Layer {
        &self.layer
    }

    /// Replace the active layer, returning the modifier flush the change implies. It flushes only
    /// when the passthrough state changed: `close` on leaving a passthrough layer (a command layer
    /// swallows the real modifier ups, so release them here), `open` on entering one (catch the app
    /// up on what is held), nothing otherwise. The one place `layer` is written.
    #[must_use = "the returned flush has to be emitted, or a held modifier is stranded down"]
    pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
        let into = into.into();
        let before_passthrough = self.layer.is_passthrough();
        let after_passthrough = into.is_passthrough();
        self.layer = into;
        match (before_passthrough, after_passthrough) {
            (true, false) => self.held.close(),
            (false, true) => self.held.open(),
            _ => Vec::new(),
        }
    }
}

/// One modifier's two physical keys. A modifier's flag is set while EITHER side is down.
#[derive(Debug, Default, Clone, Copy)]
pub struct LeftRightPair {
    pub left: bool,
    pub right: bool,
}

/// Which physical key of a left/right modifier pair.
pub enum Side {
    Left,
    Right,
}

impl LeftRightPair {
    #[must_use]
    pub const fn any_held(&self) -> bool {
        self.left || self.right
    }

    pub const fn set(&mut self, side: Side, is_down: bool) {
        match side {
            Side::Left => self.left = is_down,
            Side::Right => self.right = is_down,
        }
    }
}

/// The physical truth about which modifier keys are down. `caps_lock` is a lock, not a held key,
/// so it is not here: it changes on press and has no held down/up to replay.
#[derive(Default, Clone, Copy)]
pub struct HeldModifiers {
    pub control: LeftRightPair,
    pub meta: LeftRightPair,
    pub alt: LeftRightPair,
    pub shift: LeftRightPair,
}

impl std::fmt::Debug for HeldModifiers {
    /// Only the held modifiers, each with its side(s): `HeldModifiers { Meta(L,R), Alt(L) }`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeldModifiers {{")?;
        let mut sep = " ";
        for (name, pair) in [
            ("Control", self.control),
            ("Meta", self.meta),
            ("Alt", self.alt),
            ("Shift", self.shift),
        ] {
            let sides = match (pair.left, pair.right) {
                (true, true) => "(L,R)",
                (true, false) => "(L)",
                (false, true) => "(R)",
                (false, false) => continue,
            };
            write!(f, "{sep}{name}{sides}")?;
            sep = ", ";
        }
        f.write_str(" }")
    }
}

impl HeldModifiers {
    /// Record a modifier key's up or down. A non-modifier changes nothing.
    pub fn apply(&mut self, ev: &KeyEvent) {
        let is_down = ev.press == PressType::Down;
        match ev.key {
            Key::ControlLeft => self.control.set(Side::Left, is_down),
            Key::ControlRight => self.control.set(Side::Right, is_down),
            Key::MetaLeft => self.meta.set(Side::Left, is_down),
            Key::MetaRight => self.meta.set(Side::Right, is_down),
            Key::AltLeft => self.alt.set(Side::Left, is_down),
            Key::AltRight => self.alt.set(Side::Right, is_down),
            Key::ShiftLeft => self.shift.set(Side::Left, is_down),
            Key::ShiftRight => self.shift.set(Side::Right, is_down),
            _ => {}
        }
    }

    /// Entering a passthrough layer: a DOWN for every held key, so the app catches up.
    #[must_use]
    pub fn open(&self) -> Vec<MercuryEffect> {
        self.emit_synchronization_events(PressType::Down)
    }

    /// Leaving one: an UP for every held key, so the app forgets them.
    #[must_use]
    pub fn close(&self) -> Vec<MercuryEffect> {
        self.emit_synchronization_events(PressType::Up)
    }

    /// Emit `press` for every held key, each carrying the flags as they stand after its own
    /// change, so a shared left/right bit clears only when both sides are up.
    fn emit_synchronization_events(&self, press: PressType) -> Vec<MercuryEffect> {
        let mut shown = if press == PressType::Down {
            Self::default()
        } else {
            *self
        };
        let mut out = Vec::new();
        for key in self.held_keys() {
            shown.apply(&KeyEvent {
                key,
                press,
                flags: ModifierFlags::empty(),
            });
            out.push(emit(key, press, shown.flags()));
        }
        out
    }

    /// The modifier keys currently down, pairing each key with its field once.
    fn held_keys(&self) -> impl Iterator<Item = Key> {
        [
            (Key::ControlLeft, self.control.left),
            (Key::ControlRight, self.control.right),
            (Key::MetaLeft, self.meta.left),
            (Key::MetaRight, self.meta.right),
            (Key::AltLeft, self.alt.left),
            (Key::AltRight, self.alt.right),
            (Key::ShiftLeft, self.shift.left),
            (Key::ShiftRight, self.shift.right),
        ]
        .into_iter()
        .filter_map(|(key, held)| held.then_some(key))
    }

    /// The current modifier state as flags, for stamping on an emitted event.
    #[must_use]
    pub fn flags(&self) -> ModifierFlags {
        let mut f = ModifierFlags::empty();
        f.set(ModifierFlags::CONTROL, self.control.any_held());
        f.set(ModifierFlags::COMMAND, self.meta.any_held());
        f.set(ModifierFlags::ALT, self.alt.any_held());
        f.set(ModifierFlags::SHIFT, self.shift.any_held());
        f
    }
}

/// A keyboard event for `key`.
#[must_use]
pub const fn key(key: Key) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent {
        key,
        press: PressType::Down,
        flags: ModifierFlags::empty(),
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
