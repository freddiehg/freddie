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
//! `escape` quits from anywhere (a [`MercuryEffect::Kill`]); `return` (Enter)
//! goes back to the home layer from anywhere.
//!
//! A foreground event only records which app is frontmost; it does not change the
//! layer. Handlers either mutate the state through the path they are handed (the
//! layer transitions) or return inert [`MercuryEffect`]s. Dispatch is opaque to
//! what an effect does; performing effects is the caller's job (see the CLI and
//! the tests).
//!
//! Run it with `cargo run -p mercury`, or the tests with `cargo test -p mercury`.

use bind::{Bind, Bindings, EventTrigger};
use laserbeam::{Laserbeam, Path};

// ---------------------------------------------------------------------------
// Sources: a keyboard, and the OS reporting a newly foregrounded app.
// ---------------------------------------------------------------------------

/// A keyboard trigger for a specific key (`"a"`, `"t"`, `"escape"`, ...).
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

/// A keyboard trigger that matches any key except the global ones (`escape` and
/// `return`), so a catch-all binding still lets those bubble up to quit / go home.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnyKey;
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        !matches!(ev.key, "escape" | "return")
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
    AnyKey(AnyKey),
    Foregrounded(Foregrounded),
}
impl From<Key> for MercuryTrigger {
    fn from(k: Key) -> Self {
        Self::Key(k)
    }
}
impl From<AnyKey> for MercuryTrigger {
    fn from(a: AnyKey) -> Self {
        Self::AnyKey(a)
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
    /// Send `cmd` + this key (a refresh is `cmd`+`r`).
    Command(&'static str),
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

/// The active layer. `escape` quits from anywhere; `return` (Enter) goes home.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("escape") => kill, Key("return") => to_home)]
pub enum Layer {
    Home(HomeLayer),
    Nav(NavLayer),
    Typing(TypingLayer),
    InApp(AppLayer),
}

/// The home layer: enter nav, typing, or the in-app layer.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = HomeLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("n") => to_nav, Key("t") => to_typing, Key("i") => to_inapp)]
pub struct HomeLayer {}

/// The nav layer: foreground apps.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("c") => open_chrome,
    Key("g") => open_ghostty,
    Key("z") => open_zed,
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
#[bind(Key("r") => refresh)]
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

/// An app was foregrounded: record it. This is the only thing that changes
/// `foregrounded`, and it does not touch the layer.
const fn on_foregrounded(ev: &ForegroundEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    root.foregrounded = ev.app;
    Vec::new()
}

/// `escape`: quit, from any layer.
fn kill(_ev: &KeyEvent, _path: LayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}

/// `return`: go back to the home layer, from any layer.
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

/// Any key in typing (except the global ones): pass it through as a typed key.
fn passthru(ev: &KeyEvent, _path: TypingLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Type(ev.key)]
}

/// `r` in Chrome's in-app layer: refresh (`cmd`+`r`).
fn refresh(_ev: &KeyEvent, _path: ChromeAppPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Command("r")]
}
