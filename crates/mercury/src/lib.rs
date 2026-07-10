//! Mercury: a small, runnable demo of freddie (laserbeam + bind).
//!
//! It models a layered keyboard remapper as a pure state tree; it defines no
//! traits of its own. The state is an outer [`Mercury`] holding the currently
//! foregrounded app and a [`Layer`] it resolves into:
//!
//! - [`HomeLayer`] (the default): `n` enters nav, `t` enters typing, `i` enters
//!   the in-app layer for whatever app is foregrounded.
//! - [`NavLayer`]: `c`/`g`/`z` foreground Chrome/Ghostty/Zed and go back to home.
//!   Nav is a one-shot chooser, so `n c i r` navigates to Chrome and refreshes it.
//! - [`ResizeLayer`] (`r` from home): the arrows place the focused window, up to
//!   maximize and left and right to the halves, then it goes back to home.
//! - [`TypingLayer`]: `escape` goes home, any other key passes through.
//! - [`AppLayer`] (in-app): [`ChromeApp`] binds `r` to a refresh; every other app
//!   is [`OtherApp`], which binds nothing.
//!
//! `escape` goes back to the home layer from every sub-layer, and is a no-op in
//! home (it re-enters home). Typing binds it explicitly so its catch-all does not
//! shadow the go-home binding. From home, `q` quits, so `escape` then `q` is the
//! way out of any layer.
//!
//! A foreground event records which app is frontmost at the root, and while the
//! in-app layer is active it retargets that layer to the newly foregrounded app so
//! the active bindings follow the front app; other layers are left untouched.
//! Handlers either mutate the state through the path they are handed (the layer
//! transitions) or return inert [`MercuryEffect`]s. Dispatch is opaque to what an
//! effect does; performing effects is the caller's job (see the CLI and the
//! tests).
//!
//! Run it with `cargo run -p mercury`, or the tests with `cargo test -p mercury`.

use bind::{Bind, Bindings, EventTrigger};
pub use freddie_keys::{Key, KeyEvent, KeyPress, PressType};
use laserbeam::{Ascend, Laserbeam, Path};

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
#[derive(Debug)]
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

impl App {
    /// Maps a bundle identifier, as `freddie_app_nav` reports it, to a known app.
    /// Anything unrecognized is [`App::Other`].
    ///
    /// This is the consumer's half of the app-nav contract: the watcher hands up a
    /// string and Mercury decides which of its apps it is. Bundle ids are the
    /// stable name for an app, unlike display names, which differ depending on who
    /// is asked (`System Events` says `ghostty`, the app says `Ghostty`).
    #[must_use]
    pub fn from_bundle_id(bundle_id: &str) -> Self {
        match bundle_id {
            "com.google.Chrome" => Self::Chrome,
            "com.mitchellh.ghostty" => Self::Ghostty,
            "dev.zed.Zed" => Self::Zed,
            _ => Self::Other,
        }
    }

    /// The bundle identifier to hand `freddie_app_nav::foreground` to bring this
    /// app up. It is the same string [`from_bundle_id`](Self::from_bundle_id)
    /// matches, so the two round-trip. [`App::Other`] is not a specific app, so it
    /// has none.
    #[must_use]
    pub const fn bundle_id(self) -> Option<&'static str> {
        match self {
            Self::Chrome => Some("com.google.Chrome"),
            Self::Ghostty => Some("com.mitchellh.ghostty"),
            Self::Zed => Some("dev.zed.Zed"),
            Self::Other => None,
        }
    }
}

/// Where a window should go. Mercury's own, mirroring `freddie_windows::Placement`
/// so the model stays free of the OS crates, the way `App` is free of bundle ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Maximize,
    LeftHalf,
    RightHalf,
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
///
/// `TryInto` gives the `TryFrom<&MercuryEvent> for &SourceEvent` that dispatch
/// uses to narrow the unified event to the one a trigger cares about.
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
}

/// What a handler asks the consumer to do. Inert data; performing it is the
/// consumer's job, and it never mutates Mercury's state directly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MercuryEffect {
    /// Bring an app to the foreground.
    Foreground(App),
    /// Emit one key event, a press or a release. A chord is several of these.
    Emit(KeyEvent),
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
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

/// The resize layer: the arrows place the focused window and return home. Like
/// nav, a one-shot chooser.
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

/// The in-app layer: Chrome has bindings, everything else is ignored.
#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub enum AppLayer {
    Chrome(ChromeApp),
    Other(OtherApp),
}

impl AppLayer {
    /// The in-app variant for `app`. Only Chrome has bindings.
    #[must_use]
    pub const fn for_app(app: App) -> Self {
        match app {
            App::Chrome => Self::Chrome(ChromeApp {}),
            App::Ghostty | App::Zed | App::Other => Self::Other(OtherApp {}),
        }
    }

    /// The in-app variant for whatever app the root currently records as
    /// foregrounded. This is the "default" in-app constructor: entering the layer
    /// reads the app from root state rather than being told it.
    #[must_use]
    pub const fn for_root(root: &Mercury) -> Self {
        Self::for_app(root.foregrounded)
    }
}

#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = ChromeAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[bind(Key::KeyR.down() => refresh)]
pub struct ChromeApp {}

/// A foregrounded app Mercury has no bindings for.
#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = OtherAppPath, resolved = Resolved)]
#[binds(MercuryStruct)]
pub struct OtherApp {}

pub type LayerPath<'a> = Path<Layer, &'a mut Mercury>;
pub type HomeLayerPath<'a> = Path<HomeLayer, LayerPath<'a>>;
pub type NavLayerPath<'a> = Path<NavLayer, LayerPath<'a>>;
pub type ResizeLayerPath<'a> = Path<ResizeLayer, LayerPath<'a>>;
pub type TypingLayerPath<'a> = Path<TypingLayer, LayerPath<'a>>;
pub type AppLayerPath<'a> = Path<AppLayer, LayerPath<'a>>;
pub type ChromeAppPath<'a> = Path<ChromeApp, AppLayerPath<'a>>;
pub type OtherAppPath<'a> = Path<OtherApp, AppLayerPath<'a>>;

/// The active leaf the tree resolves to.
pub enum Resolved<'a> {
    HomeLayer(HomeLayerPath<'a>),
    NavLayer(NavLayerPath<'a>),
    ResizeLayer(ResizeLayerPath<'a>),
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

/// An app was foregrounded: record it at the root, and if we are in the in-app
/// layer, retarget it to the newly foregrounded app so the active bindings follow
/// the front app (Chrome's `r` refresh applies only while Chrome is up). Layers
/// other than in-app are left alone; foregrounding does not move you between them.
const fn on_foregrounded(ev: &ForegroundEvent, root: &mut Mercury) -> Vec<MercuryEffect> {
    root.foregrounded = ev.app;
    // Retarget the in-app layer in place rather than rebuilding `Layer::InApp`,
    // so that whatever else the variant comes to hold survives a foregrounding.
    if let Layer::InApp(in_app) = &mut root.layer {
        *in_app = AppLayer::for_app(ev.app);
    }
    Vec::new()
}

/// `q` in home: quit.
fn quit(_ev: &KeyEvent, _path: HomeLayerPath) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}

/// Put the layer back in home. The one place the home layer is entered.
fn go_home(layer: &mut LayerPath<'_>) {
    *layer.get_mut() = Layer::Home(HomeLayer {});
}

/// `escape` anywhere: go back to the home layer.
///
/// Generic over the path, so the layer enum and every node under it can bind it
/// directly. Typing has to bind it explicitly, because its catch-all would
/// otherwise shadow the layer-level binding, and now it binds this rather than a
/// wrapper that only existed to bridge the path type.
fn to_home<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, path: P) -> Vec<MercuryEffect> {
    go_home(&mut path.ascend());
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
    mercury.layer = Layer::InApp(AppLayer::for_root(mercury));
    Vec::new()
}

/// Ask for `effect` and return home.
///
/// Nav and resize are one-shot choosers: making a choice ends the layer rather
/// than leaving you in it swallowing every key that is not another choice. Generic
/// over the path, so both bind it from their own node.
///
/// The layer change is immediate; the effect is not. Foregrounding an app records
/// it only later, when the watcher reports what actually came up, so a following
/// `i` may briefly resolve the in-app layer against the old app.
/// [`on_foregrounded`] retargets it when the real event lands.
fn and_go_home<'a, P: Ascend<LayerPath<'a>>>(path: P, effect: MercuryEffect) -> Vec<MercuryEffect> {
    go_home(&mut path.ascend());
    vec![effect]
}

/// `r` in home: enter the resize layer.
fn to_resize(_ev: &KeyEvent, path: HomeLayerPath) -> Vec<MercuryEffect> {
    let mut layer = path.ascend_to::<LayerPath>();
    *layer.get_mut() = Layer::Resize(ResizeLayer {});
    Vec::new()
}

/// The arrows in resize: place the focused window and return home.
fn maximize(_ev: &KeyEvent, path: ResizeLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Place(Placement::Maximize))
}
fn left_half(_ev: &KeyEvent, path: ResizeLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Place(Placement::LeftHalf))
}
fn right_half(_ev: &KeyEvent, path: ResizeLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Place(Placement::RightHalf))
}

fn open_chrome(_ev: &KeyEvent, path: NavLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Foreground(App::Chrome))
}
fn open_ghostty(_ev: &KeyEvent, path: NavLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Foreground(App::Ghostty))
}
fn open_zed(_ev: &KeyEvent, path: NavLayerPath) -> Vec<MercuryEffect> {
    and_go_home(path, MercuryEffect::Foreground(App::Zed))
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
