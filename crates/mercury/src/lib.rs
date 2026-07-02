//! Mercury: a small demo keyboard-remapping model built on freddie.
//!
//! The state is an outer [`Mercury`] holding the currently foregrounded app and a
//! [`Layer`] it resolves into. The layers are `Nav` (the default), `Typing`, and
//! `InApp`; `InApp` carries a per-app [`AppLayer`] chosen from the foregrounded
//! app when the layer is entered.
//!
//! Handlers do two kinds of thing. Layer transitions mutate the state through the
//! path they are handed (walking up to the parent layer, or to the root to read
//! the foregrounded app). App actions return [`MercuryEffect`]s and mutate
//! nothing: opening Chrome emits `Foreground(Chrome)`, and the realistic follow-up
//! is a separate `Foreground` event dispatched back in, which is what actually
//! updates `Mercury::foregrounded`. Nothing here touches the outside world; the
//! effects are inert data a consumer would perform.

use bind::{Bind, Bindings, EventTrigger};
use laserbeam::{Laserbeam, Path};

// ---------------------------------------------------------------------------
// Sources: a keyboard, and the OS reporting a newly foregrounded app.
// ---------------------------------------------------------------------------

/// A keyboard trigger for a specific key (`"a"`, `"space"`, `"escape"`, ...).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Key(pub &'static str);
/// A fired keyboard event.
pub struct KeyEvent {
    pub key: &'static str,
}
impl EventTrigger for Key {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        self.0 == ev.key
    }
}

/// A trigger that matches any app-foregrounded event, whichever app it is.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Foregrounded;
/// A fired app-foregrounded event.
pub struct ForegroundEvent {
    pub app: App,
}
impl EventTrigger for Foregrounded {
    type Event = ForegroundEvent;
    fn is_matching(&self, _ev: &ForegroundEvent) -> bool {
        true
    }
}

/// The apps Mercury knows about. `Other` is anything it has no bindings for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum App {
    Chrome,
    Ghost,
    Tty,
    Zed,
    Other,
}

// ---------------------------------------------------------------------------
// The unified trigger, event, and effect the marker names.
// ---------------------------------------------------------------------------

/// Every trigger Mercury can register, one variant per source.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MercuryTrigger {
    Key(Key),
    Foregrounded(Foregrounded),
}
impl From<Key> for MercuryTrigger {
    fn from(k: Key) -> Self {
        Self::Key(k)
    }
}
impl From<Foregrounded> for MercuryTrigger {
    fn from(f: Foregrounded) -> Self {
        Self::Foregrounded(f)
    }
}

/// Every event Mercury can dispatch, one variant per source.
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
}
impl<'a> TryFrom<&'a MercuryEvent> for &'a KeyEvent {
    type Error = ();
    fn try_from(e: &'a MercuryEvent) -> Result<Self, ()> {
        match e {
            MercuryEvent::Key(k) => Ok(k),
            MercuryEvent::Foreground(_) => Err(()),
        }
    }
}
impl<'a> TryFrom<&'a MercuryEvent> for &'a ForegroundEvent {
    type Error = ();
    fn try_from(e: &'a MercuryEvent) -> Result<Self, ()> {
        match e {
            MercuryEvent::Foreground(f) => Ok(f),
            MercuryEvent::Key(_) => Err(()),
        }
    }
}

/// What a handler asks the consumer to do. Inert data; performing it is the
/// consumer's job, and it never mutates Mercury's state directly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MercuryEffect {
    /// Bring an app to the foreground.
    Foreground(App),
    /// Type these characters.
    Type(&'static str),
    /// Send `cmd` + this key.
    Command(&'static str),
}

/// The marker tying the trigger, event, and output types together.
pub struct MercuryStruct;
impl Bindings for MercuryStruct {
    type Trigger = MercuryTrigger;
    type Event = MercuryEvent;
    type Output = Vec<MercuryEffect>;
}

// ---------------------------------------------------------------------------
// The state tree.
// ---------------------------------------------------------------------------

/// The outer state: the foregrounded app plus the active layer.
#[derive(Laserbeam, Bind)]
#[laserbeam_root(resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Foregrounded => on_foregrounded)]
pub struct Mercury {
    pub foregrounded: App,
    #[resolve_into]
    pub layer: Layer,
}

/// The active layer. `escape` returns to `Nav` from anywhere.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("escape") => to_nav)]
pub enum Layer {
    Nav(Nav),
    Typing(Typing),
    InApp(AppLayer),
}

/// The navigation layer: switch layers and open apps.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("space") => to_typing,
    Key("i") => to_inapp,
    Key("C") => open_chrome,
    Key("G") => open_ghost,
    Key("T") => open_tty,
    Key("Z") => open_zed,
)]
pub struct Nav {}

/// The typing layer: `a`/`s`/`d`/`f` type themselves.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = TypingPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("a") => type_char,
    Key("s") => type_char,
    Key("d") => type_char,
    Key("f") => type_char,
)]
pub struct Typing {}

/// The in-app layer, one variant per foregrounded app.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub enum AppLayer {
    Chrome(Chrome),
    Ghost(Ghost),
    Tty(Tty),
    Zed(Zed),
    Other(Other),
}

impl AppLayer {
    /// The in-app variant for the foregrounded app.
    #[must_use]
    pub const fn for_app(app: App) -> Self {
        match app {
            App::Chrome => Self::Chrome(Chrome {}),
            App::Ghost => Self::Ghost(Ghost {}),
            App::Tty => Self::Tty(Tty {}),
            App::Zed => Self::Zed(Zed {}),
            App::Other => Self::Other(Other {}),
        }
    }
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = ChromePath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("r") => command)]
pub struct Chrome {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = GhostPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("d") => command)]
pub struct Ghost {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = TtyPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("d") => command)]
pub struct Tty {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = ZedPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("d") => command)]
pub struct Zed {}

/// A foregrounded app Mercury has no bindings for.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = OtherPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub struct Other {}

pub type LayerPath<'a> = Path<Layer, &'a mut Mercury>;
pub type NavPath<'a> = Path<Nav, LayerPath<'a>>;
pub type TypingPath<'a> = Path<Typing, LayerPath<'a>>;
pub type AppLayerPath<'a> = Path<AppLayer, LayerPath<'a>>;
pub type ChromePath<'a> = Path<Chrome, AppLayerPath<'a>>;
pub type GhostPath<'a> = Path<Ghost, AppLayerPath<'a>>;
pub type TtyPath<'a> = Path<Tty, AppLayerPath<'a>>;
pub type ZedPath<'a> = Path<Zed, AppLayerPath<'a>>;
pub type OtherPath<'a> = Path<Other, AppLayerPath<'a>>;

/// The active leaf the tree resolves to.
pub enum Resolved<'a> {
    Nav(NavPath<'a>),
    Typing(TypingPath<'a>),
    Chrome(ChromePath<'a>),
    Ghost(GhostPath<'a>),
    Tty(TtyPath<'a>),
    Zed(ZedPath<'a>),
    Other(OtherPath<'a>),
}

impl Default for Mercury {
    fn default() -> Self {
        Self {
            foregrounded: App::Other,
            layer: Layer::Nav(Nav {}),
        }
    }
}

impl Mercury {
    /// Dispatches an event, returning the handler's effects, or `None` when the
    /// active state binds nothing for it.
    #[must_use]
    pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
        bind::dispatch::<MercuryStruct, Self>(self, event)
    }
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// An app was foregrounded: record it. This is the follow-up to a `Foreground`
/// effect, and the only thing that changes `foregrounded`.
const fn on_foregrounded(ev: &ForegroundEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    root.foregrounded = ev.app;
    Vec::new()
}

/// `escape`: back to the nav layer, from any layer.
fn to_nav(_ev: &KeyEvent, mut path: LayerPath) -> Vec<MercuryEffect> {
    *path.get_mut() = Layer::Nav(Nav {});
    Vec::new()
}

/// `space` in nav: enter the typing layer.
fn to_typing(_ev: &KeyEvent, path: NavPath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Typing(Typing {});
    Vec::new()
}

/// `i` in nav: enter the in-app layer for the currently foregrounded app.
fn to_inapp(_ev: &KeyEvent, path: NavPath) -> Vec<MercuryEffect> {
    let mercury = path.into_parent().into_parent();
    mercury.layer = Layer::InApp(AppLayer::for_app(mercury.foregrounded));
    Vec::new()
}

fn open_chrome(_ev: &KeyEvent, _path: NavPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Chrome)]
}
fn open_ghost(_ev: &KeyEvent, _path: NavPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Ghost)]
}
fn open_tty(_ev: &KeyEvent, _path: NavPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Tty)]
}
fn open_zed(_ev: &KeyEvent, _path: NavPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Zed)]
}

/// `a`/`s`/`d`/`f` in typing: type the key.
fn type_char(ev: &KeyEvent, _path: TypingPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Type(ev.key)]
}

/// A per-app key: send `cmd` + the key. Generic over the app's path.
fn command<P>(ev: &KeyEvent, _path: P) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Command(ev.key)]
}

#[cfg(test)]
mod tests {
    use super::{App, KeyEvent, Layer, Mercury, MercuryEffect, MercuryEvent, ForegroundEvent, AppLayer};

    fn key(k: &'static str) -> MercuryEvent {
        MercuryEvent::Key(KeyEvent { key: k })
    }
    fn foreground(app: App) -> MercuryEvent {
        MercuryEvent::Foreground(ForegroundEvent { app })
    }

    #[test]
    fn starts_in_nav() {
        let m = Mercury::default();
        assert!(matches!(m.layer, Layer::Nav(_)));
        assert_eq!(m.foregrounded, App::Other);
    }

    #[test]
    fn space_enters_typing_with_no_effect() {
        let mut m = Mercury::default();
        assert_eq!(m.handle(&key("space")), Some(vec![]));
        assert!(matches!(m.layer, Layer::Typing(_)));
    }

    #[test]
    fn typing_types_characters() {
        let mut m = Mercury::default();
        m.handle(&key("space"));
        assert_eq!(m.handle(&key("a")), Some(vec![MercuryEffect::Type("a")]));
        assert_eq!(m.handle(&key("f")), Some(vec![MercuryEffect::Type("f")]));
    }

    #[test]
    fn escape_returns_to_nav() {
        let mut m = Mercury::default();
        m.handle(&key("space"));
        assert!(matches!(m.layer, Layer::Typing(_)));
        assert_eq!(m.handle(&key("escape")), Some(vec![]));
        assert!(matches!(m.layer, Layer::Nav(_)));
    }

    #[test]
    fn opening_an_app_emits_an_effect_but_does_not_mutate_state() {
        let mut m = Mercury::default();
        assert_eq!(
            m.handle(&key("C")),
            Some(vec![MercuryEffect::Foreground(App::Chrome)])
        );
        // The effect is inert: still in nav, still foregrounding nothing known.
        assert!(matches!(m.layer, Layer::Nav(_)));
        assert_eq!(m.foregrounded, App::Other);
    }

    #[test]
    fn foreground_event_updates_the_foregrounded_app() {
        let mut m = Mercury::default();
        assert_eq!(m.handle(&foreground(App::Chrome)), Some(vec![]));
        assert_eq!(m.foregrounded, App::Chrome);
        // Handled at the root regardless of layer.
        assert!(matches!(m.layer, Layer::Nav(_)));
    }

    #[test]
    fn in_app_uses_the_foregrounded_app() {
        let mut m = Mercury::default();
        m.handle(&foreground(App::Zed));
        m.handle(&key("i"));
        assert!(matches!(m.layer, Layer::InApp(AppLayer::Zed(_))));
        assert_eq!(m.handle(&key("d")), Some(vec![MercuryEffect::Command("d")]));
    }

    #[test]
    fn chrome_rebinds_r_to_command_r() {
        let mut m = Mercury::default();
        m.handle(&foreground(App::Chrome));
        m.handle(&key("i"));
        assert_eq!(m.handle(&key("r")), Some(vec![MercuryEffect::Command("r")]));
    }

    // The realistic loop: open Chrome (effect only), the OS reports it foregrounded
    // (a separate event, which updates state), then the in-app layer reflects it.
    #[test]
    fn open_then_foreground_then_in_app() {
        let mut m = Mercury::default();

        assert_eq!(
            m.handle(&key("C")),
            Some(vec![MercuryEffect::Foreground(App::Chrome)])
        );
        assert_eq!(m.foregrounded, App::Other); // the effect did not mutate state

        assert_eq!(m.handle(&foreground(App::Chrome)), Some(vec![]));
        assert_eq!(m.foregrounded, App::Chrome);

        assert_eq!(m.handle(&key("i")), Some(vec![]));
        assert!(matches!(m.layer, Layer::InApp(AppLayer::Chrome(_))));

        assert_eq!(m.handle(&key("r")), Some(vec![MercuryEffect::Command("r")]));
    }

    #[test]
    fn unrelated_foregrounded_app_has_no_in_app_bindings() {
        let mut m = Mercury::default();
        m.handle(&foreground(App::Other));
        m.handle(&key("i"));
        assert!(matches!(m.layer, Layer::InApp(AppLayer::Other(_))));
        // Other binds nothing, and no ancestor binds `d`.
        assert_eq!(m.handle(&key("d")), None);
    }

    #[test]
    fn unbound_key_returns_none() {
        let mut m = Mercury::default();
        assert_eq!(m.handle(&key("x")), None);
    }

    #[test]
    fn accumulate_gathers_the_active_nav_triggers() {
        use super::{Foregrounded, Key, MercuryStruct, MercuryTrigger};
        use std::collections::HashSet;

        let m = Mercury::default();
        let set = bind::accumulate::<MercuryStruct, _>(&m).unwrap();
        assert_eq!(
            set,
            HashSet::from([
                MercuryTrigger::Foregrounded(Foregrounded),
                MercuryTrigger::Key(Key("escape")),
                MercuryTrigger::Key(Key("space")),
                MercuryTrigger::Key(Key("i")),
                MercuryTrigger::Key(Key("C")),
                MercuryTrigger::Key(Key("G")),
                MercuryTrigger::Key(Key("T")),
                MercuryTrigger::Key(Key("Z")),
            ])
        );
    }
}
