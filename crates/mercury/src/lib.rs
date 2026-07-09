//! Mercury: a small, runnable demo of freddie (laserbeam + bind).
//!
//! It models a layered keyboard remapper as a pure state tree; it defines no
//! traits of its own. The state is an outer [`Mercury`] holding the currently
//! foregrounded app and a [`Layer`] it resolves into:
//!
//! - [`HomeLayer`] (the default): `n` enters nav, `t` enters typing, `i` enters
//!   the in-app layer for whatever app is foregrounded.
//! - [`NavLayer`]: `c`/`g`/`z` foreground Chrome/Ghostty/Zed.
//! - [`TypingLayer`]: any key passes through as a typed key.
//! - [`AppLayer`] (in-app): [`ChromeApp`] binds `r` to a refresh; every other app
//!   is [`OtherApp`], which binds nothing.
//!
//! `escape` goes back to the home layer from the nav and in-app layers. In home
//! and typing it passes through instead: home binds it explicitly, and typing's
//! catch-all shadows the go-home binding.
//!
//! A foreground event only records which app is frontmost; it does not change the
//! layer. Handlers either mutate the state through the path they are handed (the
//! layer transitions) or return inert [`MercuryEffect`]s. Dispatch is opaque to
//! what an effect does; performing effects is the caller's job (see the CLI and
//! the tests).
//!
//! Run it with `cargo run -p mercury`, or the tests with `cargo test -p mercury`.

use bind::{Bind, Bindings, EventTrigger};
pub use freddie_keys::{Key, KeyEvent, KeyPress, PressType};
use laserbeam::{Laserbeam, Path};

// ---------------------------------------------------------------------------
// Sources: a keyboard, and the OS reporting a newly foregrounded app.
// ---------------------------------------------------------------------------

// A specific key is its own trigger: `Key::KeyR` binds that key. The type and
// its `EventTrigger` impl live in `freddie_keys`, so no wrapper is needed here.

/// A keyboard trigger that matches every key, on either press.
///
/// A catch-all: when a layer binds it, it shadows an ancestor's binding for the
/// same key (dispatch is leafward). There is no ordering between it and a
/// specific-key trigger, so binding both on one active path is a shadow, not a
/// conflict.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnyKey;
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, _ev: &KeyEvent) -> bool {
        true
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
    Ghostty,
    Zed,
    Other,
}

// ---------------------------------------------------------------------------
// The unified trigger, event, and effect the marker names.
// ---------------------------------------------------------------------------

/// Every trigger Mercury can register, one variant per source.
#[derive(Clone, PartialEq, Eq, Hash, Debug, derive_more::From)]
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyKey(AnyKey),
    Foregrounded(Foregrounded),
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
    /// Emit one key event, a press or a release. A chord is several of these.
    Emit(KeyEvent),
    /// Quit the program. The effect handler performs this by exiting.
    Kill,
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

/// The active layer. `escape` goes home from a sub-layer (home passes it through).
#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
}

/// The home layer: enter a layer, quit with `q`, or pass `escape` through.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = HomeLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyQ.down() => quit,
    Key::Escape.down() => passthru,
)]
pub struct HomeLayer {}

/// The nav layer: foreground apps.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}

/// The typing layer: any key passes through as a typed key.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = TypingLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(AnyKey => passthru)]
pub struct TypingLayer {}

/// The in-app layer: Chrome has bindings, everything else is ignored.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub enum AppLayer {
    Chrome(ChromeApp),
    Other(OtherApp),
}

impl AppLayer {
    /// The in-app variant for the foregrounded app. Only Chrome has bindings.
    #[must_use]
    pub const fn for_app(app: App) -> Self {
        match app {
            App::Chrome => Self::Chrome(ChromeApp {}),
            App::Ghostty | App::Zed | App::Other => Self::Other(OtherApp {}),
        }
    }
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = ChromeAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key::KeyR.down() => refresh)]
pub struct ChromeApp {}

/// A foregrounded app Mercury has no bindings for.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = OtherAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub struct OtherApp {}

pub type LayerPath<'a> = Path<Layer, &'a mut Mercury>;
pub type HomeLayerPath<'a> = Path<HomeLayer, LayerPath<'a>>;
pub type NavLayerPath<'a> = Path<NavLayer, LayerPath<'a>>;
pub type TypingLayerPath<'a> = Path<TypingLayer, LayerPath<'a>>;
pub type AppLayerPath<'a> = Path<AppLayer, LayerPath<'a>>;
pub type ChromeAppPath<'a> = Path<ChromeApp, AppLayerPath<'a>>;
pub type OtherAppPath<'a> = Path<OtherApp, AppLayerPath<'a>>;

/// The active leaf the tree resolves to.
pub enum Resolved<'a> {
    HomeLayer(HomeLayerPath<'a>),
    NavLayer(NavLayerPath<'a>),
    TypingLayer(TypingLayerPath<'a>),
    ChromeApp(ChromeAppPath<'a>),
    OtherApp(OtherAppPath<'a>),
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
    /// Dispatches one event, returning the handler's effects, or `None` when the
    /// active state binds nothing for it.
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

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// An app was foregrounded: record it. This is the only thing that changes
/// `foregrounded`, and it does not touch the layer.
const fn on_foregrounded(ev: &ForegroundEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    root.foregrounded = ev.app;
    Vec::new()
}

/// `q` in home: quit.
fn quit(_ev: &KeyEvent, _path: HomeLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}

/// `escape` in a sub-layer: go back to the home layer.
fn to_home(_ev: &KeyEvent, mut path: LayerPath) -> Vec<MercuryEffect> {
    *path.get_mut() = Layer::Home(HomeLayer {});
    Vec::new()
}

/// `n` in home: enter the nav layer.
fn to_nav(_ev: &KeyEvent, path: HomeLayerPath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Nav(NavLayer {});
    Vec::new()
}

/// `t` in home: enter the typing layer.
fn to_typing(_ev: &KeyEvent, path: HomeLayerPath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Typing(TypingLayer {});
    Vec::new()
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
fn to_inapp(_ev: &KeyEvent, path: HomeLayerPath) -> Vec<MercuryEffect> {
    let mercury = path.into_parent().into_parent();
    let app = mercury.foregrounded;
    mercury.layer = Layer::InApp(AppLayer::for_app(app));
    Vec::new()
}

fn open_chrome(_ev: &KeyEvent, _path: NavLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Chrome)]
}
fn open_ghostty(_ev: &KeyEvent, _path: NavLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Ghostty)]
}
fn open_zed(_ev: &KeyEvent, _path: NavLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Foreground(App::Zed)]
}

fn passthru<P>(ev: &KeyEvent, _path: P) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}

fn refresh(_ev: &KeyEvent, _path: ChromeAppPath) -> Vec<MercuryEffect> {
    let emit = |key, press| MercuryEffect::Emit(KeyEvent { key, press });
    vec![
        emit(Key::MetaLeft, PressType::Down),
        emit(Key::KeyR, PressType::Down),
        emit(Key::KeyR, PressType::Up),
        emit(Key::MetaLeft, PressType::Up),
    ]
}
