//! Mercury: a small, runnable demo of freddie (laserbeam + bind).
//!
//! It models a layered keyboard remapper as a pure state tree; it defines no
//! traits of its own. The state is an outer [`Mercury`] holding the currently
//! foregrounded app and a [`Layer`] it resolves into:
//!
//! - [`HomeLayer`] (the default): `n` enters nav, `space` enters typing.
//! - [`NavLayer`]: `c`/`g`/`z` open Chrome/Ghostty/Zed.
//! - [`TypingLayer`]: `a`/`s`/`d`/`f` type themselves.
//! - [`AppLayer`] (in-app): one variant per app. [`ChromeApp`] rebinds `r` to
//!   `cmd`+`r`; [`GhosttyApp`] and [`ZedApp`] rebind `d` to `cmd`+`d`;
//!   [`OtherApp`] binds nothing.
//!
//! `escape` returns to home from anywhere.
//!
//! Handlers either mutate the state through the path they are handed (the layer
//! transitions) or return inert [`MercuryEffect`]s (typing a letter, opening an
//! app, sending a command). Dispatch is opaque to what an effect does. Driving
//! effects — and turning `Foreground(app)` into the follow-up foreground event,
//! which can fail if the app is missing — is the caller's job, via an event loop
//! and handler functions (see the CLI and the tests).
//!
//! Run it with `cargo run -p mercury` (one key per line), or the tests with
//! `cargo test -p mercury`.

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
    Ghostty,
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
    /// A letter was typed.
    Type(&'static str),
    /// Send `cmd` + this key.
    Command(&'static str),
    /// Exit the program. The effect handler performs this by exiting; a
    /// killswitch (a timer now, a key later) asks for it.
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

/// The active layer. `escape` returns to the home layer from anywhere.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("escape") => to_home)]
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
}

/// The home layer: enter nav or typing.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = HomeLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("n") => to_nav, Key("space") => to_typing)]
pub struct HomeLayer {}

/// The nav layer: open apps.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("c") => open_chrome,
    Key("g") => open_ghostty,
    Key("z") => open_zed,
)]
pub struct NavLayer {}

/// The typing layer: `a`/`s`/`d`/`f` type themselves.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = TypingLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("a") => type_char,
    Key("s") => type_char,
    Key("d") => type_char,
    Key("f") => type_char,
)]
pub struct TypingLayer {}

/// The in-app layer, one variant per foregrounded app.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub enum AppLayer {
    Chrome(ChromeApp),
    Ghostty(GhosttyApp),
    Zed(ZedApp),
    Other(OtherApp),
}

impl AppLayer {
    /// The in-app variant for the foregrounded app.
    #[must_use]
    pub const fn for_app(app: App) -> Self {
        match app {
            App::Chrome => Self::Chrome(ChromeApp {}),
            App::Ghostty => Self::Ghostty(GhosttyApp {}),
            App::Zed => Self::Zed(ZedApp {}),
            App::Other => Self::Other(OtherApp {}),
        }
    }
}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = ChromeAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("r") => command)]
pub struct ChromeApp {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = GhosttyAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("d") => command)]
pub struct GhosttyApp {}

#[derive(Laserbeam, Bind)]
#[laserbeam(path = ZedAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("d") => command)]
pub struct ZedApp {}

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
pub type GhosttyAppPath<'a> = Path<GhosttyApp, AppLayerPath<'a>>;
pub type ZedAppPath<'a> = Path<ZedApp, AppLayerPath<'a>>;
pub type OtherAppPath<'a> = Path<OtherApp, AppLayerPath<'a>>;

/// The active leaf the tree resolves to.
pub enum Resolved<'a> {
    HomeLayer(HomeLayerPath<'a>),
    NavLayer(NavLayerPath<'a>),
    TypingLayer(TypingLayerPath<'a>),
    ChromeApp(ChromeAppPath<'a>),
    GhosttyApp(GhosttyAppPath<'a>),
    ZedApp(ZedAppPath<'a>),
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
pub const fn key(key: &'static str) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent { key })
}

/// An app-foregrounded event for `app`.
#[must_use]
pub const fn foreground(app: App) -> MercuryEvent {
    MercuryEvent::Foreground(ForegroundEvent { app })
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// An app was foregrounded: record it and enter its in-app layer. This is the
/// follow-up to a `Foreground` effect, and the only thing that changes
/// `foregrounded`.
const fn on_foregrounded(ev: &ForegroundEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    root.foregrounded = ev.app;
    root.layer = Layer::InApp(AppLayer::for_app(ev.app));
    Vec::new()
}

/// `escape`: back to the home layer, from any layer.
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

/// `space` in home: enter the typing layer.
fn to_typing(_ev: &KeyEvent, path: HomeLayerPath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Typing(TypingLayer {});
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

/// `a`/`s`/`d`/`f` in typing: type the key.
fn type_char(ev: &KeyEvent, _path: TypingLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Type(ev.key)]
}

/// A per-app key: send `cmd` + the key. Generic over the app's path.
fn command<P>(ev: &KeyEvent, _path: P) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Command(ev.key)]
}
