//! Mercury: a small, runnable demo of freddie (laserbeam + bind).
//!
//! It models a layered keyboard remapper. The state is an outer [`Mercury`]
//! holding the currently foregrounded app and a [`Layer`] it resolves into:
//!
//! - `Home` (the default): `n` enters nav, `space` enters typing.
//! - `Nav`: `c`/`g`/`t`/`z` open Chrome/Ghostty/TTY/Zed.
//! - `Typing`: `a`/`s`/`d`/`f` type themselves.
//! - `InApp`: a per-app [`AppLayer`]. Chrome rebinds `r` to `cmd`+`r`; the
//!   terminals rebind `d` to `cmd`+`d`; an unknown app binds nothing.
//!
//! `escape` returns to `Home` from anywhere.
//!
//! Handlers either mutate the state through the path they are handed (the layer
//! transitions) or return inert [`MercuryEffect`]s (typing a letter, opening an
//! app, sending a command). Opening an app emits `Foreground(app)` and mutates
//! nothing; the app coming up is a separate `Foreground` event, which is the only
//! thing that records the foregrounded app and enters its in-app layer. Turning
//! the effect into that event is the effect handler's job, and it can fail (you
//! might not have the app), so dispatch stays oblivious to it. See [`drive`] and
//! [`EffectHandler`].
//!
//! Run it with `cargo run -p mercury` (one key per line), or the tests with
//! `cargo test -p mercury`.

use std::collections::VecDeque;

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
    /// A letter was typed.
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

/// The active layer. `escape` returns to `Home` from anywhere.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = LayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("escape") => to_home)]
pub enum Layer {
    Home(Home),
    Nav(Nav),
    Typing(Typing),
    InApp(AppLayer),
}

/// The home layer: enter nav or typing.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = HomePath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key("n") => to_nav, Key("space") => to_typing)]
pub struct Home {}

/// The nav layer: open apps.
#[derive(Laserbeam, Bind)]
#[laserbeam(path = NavPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(
    Key("c") => open_chrome,
    Key("g") => open_ghost,
    Key("t") => open_tty,
    Key("z") => open_zed,
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
pub type HomePath<'a> = Path<Home, LayerPath<'a>>;
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
    Home(HomePath<'a>),
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
            layer: Layer::Home(Home {}),
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

/// Performs the effects a dispatch produces.
///
/// What an effect does is entirely the handler's business; dispatch is opaque to
/// it. Performing an effect may cause more effects — foregrounding an app makes
/// it the foreground app, which the handler turns into a follow-up event whose
/// effects come back here. [`drive`] feeds those back in turn.
pub trait EffectHandler {
    /// Perform `effect`, returning any further effects it causes. It may dispatch
    /// follow-up events on `state` and return the effects they produce.
    fn handle(&mut self, effect: &MercuryEffect, state: &mut Mercury) -> Vec<MercuryEffect>;
}

/// Dispatch `event`, then drain the effects it produces through `handler`,
/// handling whatever further effects those cause, until none remain.
///
/// Dispatch never knows that an effect can cause an event; that lives entirely in
/// the handler. This loop only pops effects and feeds back what the handler adds.
pub fn drive(state: &mut Mercury, event: &MercuryEvent, handler: &mut impl EffectHandler) {
    let Some(effects) = state.handle(event) else {
        return;
    };
    let mut pending: VecDeque<MercuryEffect> = effects.into();
    while let Some(effect) = pending.pop_front() {
        pending.extend(handler.handle(&effect, state));
    }
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
    *path.get_mut() = Layer::Home(Home {});
    Vec::new()
}

/// `n` in home: enter the nav layer.
fn to_nav(_ev: &KeyEvent, path: HomePath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Nav(Nav {});
    Vec::new()
}

/// `space` in home: enter the typing layer.
fn to_typing(_ev: &KeyEvent, path: HomePath) -> Vec<MercuryEffect> {
    let mut layer = path.into_parent();
    *layer.get_mut() = Layer::Typing(Typing {});
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
