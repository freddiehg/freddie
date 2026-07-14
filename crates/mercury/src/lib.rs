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
//! - [`AppLayer`] (in-app): it stores NO app. [`app_data`] reads `root.foregrounded`
//!   on every dispatch and builds the app's level from it, so there is one copy of
//!   the foregrounded app and nothing to keep in sync. [`ChromeApp`] binds `r` to a
//!   refresh; [`GhosttyApp`] binds `j`/`k` to tmux's previous and next window and
//!   `1`-`0` to windows one through ten. An app with no bindings gets no level at
//!   all: `app_data` returns `None` for it.
//!
//! A layer stays only if its actions make sense to do repeatedly. Walking panes
//! and refreshing a page do, so the in-app layers stay put. Choosing an app or a
//! window placement does not, so nav and resize are one-shot choosers that return
//! home. See [`and_go_home`].
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

use bind::{Bind, Bindings, EventTrigger, Node};
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
    /// Tap `key` while `modifiers` are held. The chord.
    ///
    /// The modifiers are pressed, the key is tapped, and they are released, so a
    /// key cannot carry a modifier that was never pressed. Prefer this to a
    /// hand-written sequence of [`Emit`](Self::Emit)s.
    Tap { modifiers: Vec<Key>, key: Key },
    /// Emit one raw key event, a press or a release on its own.
    ///
    /// The escape hatch, for the one case that is genuinely a lone half of a
    /// keypress: passing a key through, where the model sees a down and an up as
    /// separate events and re-emits each. Building a chord out of these is a bug
    /// waiting to happen; use [`Tap`](Self::Tap).
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

/// The in-app layer. It stores NO app: `root.foregrounded` is the only copy, and
/// [`app_data`] builds the app's level from it on every dispatch. There is nothing
/// to keep in sync and nothing to go stale.
#[derive(Laserbeam, Bind, Debug)]
#[laserbeam(path = AppLayerPath, resolved = Resolved)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[derive(Default)]
pub struct AppLayer {}

/// The app's level, which is not in the tree. Several possible levels, so the data is
/// an enum; an app with no bindings is not a variant, and [`app_data`] returns `None`
/// for it.
#[derive(Bind, Debug)]
#[derived_node(parent = AppLayerPath)]
#[binds(MercuryStruct)]
pub enum AppData {
    Chrome(ChromeApp),
    Ghostty(GhosttyApp),
}

/// Reads `root.foregrounded`, the only copy, and builds the level for it.
///
/// A shared reference, so it cannot mutate: it derives, it does not act. `Zed` and
/// `Other` bind nothing, so they get no level and no struct.
fn app_data(path: &AppLayerPath) -> Option<AppData> {
    match path.parent().parent().foregrounded {
        App::Chrome => Some(AppData::Chrome(ChromeApp {})),
        App::Ghostty => Some(AppData::Ghostty(GhosttyApp {})),
        App::Zed | App::Other => None,
    }
}

/// Chrome's level. A unit for now: mercury tracks nothing per app. It stops being one
/// when it carries something (a tab name).
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
const fn on_foregrounded(ev: &ForegroundEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    // The only copy. The in-app layer holds no app, so there is nothing to resync and
    // nothing that can go stale: `app_data` rebuilds the app's level from this on every
    // dispatch.
    node.parent.foregrounded = ev.app;
    Vec::new()
}

/// `q` in home: quit.
fn quit(_ev: &KeyEvent, _node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
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
fn to_home<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    go_home(&mut node.parent.ascend());
    Vec::new()
}

/// `n` in home: enter the nav layer.
fn to_nav(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let mut layer = node.parent.into_parent();
    *layer.get_mut() = Layer::Nav(NavLayer {});
    Vec::new()
}

/// `t` in home: enter the typing layer.
fn to_typing(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let mut layer = node.parent.into_parent();
    *layer.get_mut() = Layer::Typing(TypingLayer {});
    Vec::new()
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
fn to_inapp(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let mercury = node.parent.into_parent().into_parent();
    mercury.layer = Layer::InApp(AppLayer {});
    Vec::new()
}

/// Ask for `effect` and return home.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's
/// panes and refreshing Chrome do, so the in-app layers stay. Choosing an app or a
/// window placement does not: repeating it is a no-op, and anything else is a
/// different choice. So nav and resize are one-shot choosers, and this is how they
/// leave.
///
/// Generic over the path, so both bind it from their own node.
///
/// The layer change is immediate; the effect is not. Foregrounding an app records
/// it only later, when the watcher reports what actually came up, so a following
/// `i` may briefly resolve the in-app layer against the old app.
/// [`on_foregrounded`] retargets it when the real event lands.
fn and_go_home<'a, P: Ascend<LayerPath<'a>>>(
    path: P,
    effects: Vec<MercuryEffect>,
) -> Vec<MercuryEffect> {
    go_home(&mut path.ascend());
    effects
}

/// `r` in home: enter the resize layer.
fn to_resize(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let mut layer = node.parent.ascend_to::<LayerPath>();
    *layer.get_mut() = Layer::Resize(ResizeLayer {});
    Vec::new()
}

/// The arrows in resize: place the focused window and return home.
fn maximize(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::Maximize)])
}
fn left_half(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Place(Placement::LeftHalf)])
}
fn right_half(_ev: &KeyEvent, node: Node<ResizeLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(
        node.parent,
        vec![MercuryEffect::Place(Placement::RightHalf)],
    )
}

fn open_chrome(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Chrome)])
}
fn open_ghostty(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Ghostty)])
}
fn open_zed(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Zed)])
}

fn passthru<P>(ev: &KeyEvent, _path: P) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}

/// Tap `key` while `modifiers` are held.
fn tap(modifiers: &[Key], key: Key) -> MercuryEffect {
    MercuryEffect::Tap {
        modifiers: modifiers.to_vec(),
        key,
    }
}

/// A tmux command: the `ctrl-a` prefix, then the command key.
///
/// Two taps rather than one chord, because the prefix has to be let go before the
/// command or tmux sees `ctrl-p` rather than `p`. Which is now what the shape says,
/// rather than something the order of six raw events has to get right.
fn tmux(modifiers: &[Key], command: Key) -> Vec<MercuryEffect> {
    vec![tap(&[Key::ControlLeft], Key::KeyA), tap(modifiers, command)]
}

/// `j` in Ghostty: tmux's previous window. Stays, because walking windows repeats.
fn previous_window(_ev: &KeyEvent, _node: GhosttyAppNode) -> Vec<MercuryEffect> {
    tmux(&[], Key::KeyP)
}

/// `k` in Ghostty: tmux's next window.
fn next_window(_ev: &KeyEvent, _node: GhosttyAppNode) -> Vec<MercuryEffect> {
    tmux(&[], Key::KeyN)
}

/// The digits in Ghostty: jump straight to a tmux window, then go home.
///
/// The window is chosen with the digit's *shifted* symbol, because that is what
/// the tmux config binds: `!` through `)` select windows 1 through 10, while the
/// bare digits select window *indices* and so cannot reach the tenth. `1` sends
/// `ctrl-a !` and `0` sends `ctrl-a )`.
///
/// Jumping to a window is a choice rather than something you repeat, so it leaves
/// the layer. See [`and_go_home`].
macro_rules! select_window {
    ($($handler:ident => $digit:ident),* $(,)?) => {$(
        fn $handler(_ev: &KeyEvent, node: GhosttyAppNode) -> Vec<MercuryEffect> {
            and_go_home(node.parent, tmux(&[Key::ShiftLeft], Key::$digit))
        }
    )*};
}

select_window! {
    window_1 => Num1,
    window_2 => Num2,
    window_3 => Num3,
    window_4 => Num4,
    window_5 => Num5,
    window_6 => Num6,
    window_7 => Num7,
    window_8 => Num8,
    window_9 => Num9,
    window_0 => Num0,
}

/// `r` in Chrome: cmd-r, a refresh.
fn refresh(_ev: &KeyEvent, _node: ChromeAppNode) -> Vec<MercuryEffect> {
    vec![tap(&[Key::MetaLeft], Key::KeyR)]
}
