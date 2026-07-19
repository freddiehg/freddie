//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use std::time::Duration;

use bind::Bind;
use freddie::{KeySequence, TimerFired, TimerGuard, timer_effect_and_guard};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};
use laserbeam::PathMut;

// The derive generates a call to each named handler at its node's definition site below, so
// every handler has to be in scope here. A glob keeps this in step with the handler set instead
// of a name-by-name list that drifts.
use crate::effect::emit;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{
    AnyKey, App, ForegroundEvent, Foregrounded, MercuryEffect, MercuryEvent, MercuryStruct, Quit,
    TabEvent, Tabbed,
};

mod app;
mod home;
mod nav;
mod resize;
mod typing;

pub use app::{AppData, AppLayer, ChromeApp, GhosttyApp};
pub use home::HomeLayer;
pub use nav::NavLayer;
pub use resize::ResizeLayer;
pub use typing::TypingLayer;

/// How long a chooser layer sits idle before returning home.
pub const RETURN_TO_HOME_TIMEOUT: Duration = Duration::from_secs(10);

/// Arm the return-to-home timer a layer holds: the guard cancels it on drop, and the effect
/// schedules it. It fires after [`RETURN_TO_HOME_TIMEOUT`], and the layer that set it binds that
/// firing home, matching on the guard it still holds.
fn arm_return_home() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, |id| {
        MercuryEvent::Timer(TimerFired(id))
    });
    (guard, MercuryEffect::Timer(effect))
}

/// How long the overlay stays up before its hide timer fires.
pub const OVERLAY_DWELL: Duration = Duration::from_secs(10);

/// How long a `jk` run waits for its next key before what it swallowed types itself.
///
/// It bounds how long a `j` stays invisible, so shorter is better, but it has to cover a
/// deliberately typed `jk` (down, up, down) rather than only a rolled one, which is far faster.
pub const JK_TIMEOUT: Duration = Duration::from_millis(200);

/// Arm a run's window: the guard cancels it on drop, the effect schedules it. The delay is the
/// run's own, read off the sequence, so this does not restate the policy.
///
/// `pub(crate)` where `arm_return_home` is private, because the root's handlers call this one and
/// they are not children of this module.
pub(crate) fn arm_jk_timeout(window: Duration) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(window, |id| MercuryEvent::Timer(TimerFired(id)));
    (guard, MercuryEffect::Timer(effect))
}

#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Quit => quit,
    // Only this run's window: a firing from a run that has since ended matches nothing, so the
    // handler never sees it.
    |mercury_path| mercury_path.typing_state.jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    // Only the showing that is up: a dwell from one already replaced matches nothing.
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    AnyKey => maybe_pass_through,
)]
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    /// The state the passthrough (typing) behavior needs. See [`TypingState`].
    pub typing_state: TypingState,
    /// The overlay currently up, if any: the guard for its pending hide. The overlay is an
    /// external window driven by effects, so this is its only trace in the model, held at the root
    /// because there is one overlay across all layers. The root's binding names it, so a firing
    /// from a showing that was replaced matches nothing.
    ///
    /// Private for the reason `layer` is: the effects a change implies come back from the method
    /// that made it.
    overlay: Option<TimerGuard>,
    /// The active layer. Private, and written only through [`set_layer`](Mercury::set_layer), so
    /// no transition can change the layer without going through the modifier flush.
    #[resolve_into]
    layer: Layer,
}

/// What mercury knows about the frontmost Chrome.
///
/// It exists only inside [`ForegroundedApp::Chrome`], so there is no tab URL to be meaningless
/// while Finder is up, and nothing to clear when Chrome goes away: the value goes with it.
#[derive(Debug, Default)]
pub struct ForegroundedChrome {
    /// The front tab's URL, raw, as the tab source sent it.
    ///
    /// `None` until that source reports, which is also the state right after Chrome comes up: the
    /// active tab is Chrome's to know, and no app-activation event carries it. A site level
    /// resolves only once this is `Some`, so a key pressed in the gap is unbound rather than aimed
    /// at whatever site was there before.
    ///
    /// A `String` rather than a parsed URL: [`Site::from_url`] matches a host, which is a scan of a
    /// short string, and keeping it raw leaves the whole URL for handlers that want it.
    pub url: Option<String>,
}

/// The frontmost app, and whatever mercury knows about it.
///
/// [`App`] stays the identity that events and effects speak, because neither the watcher reporting
/// an activation nor an effect asking for one knows anything about a tab. This is the same set of
/// apps with the state hung off the one that has any.
#[derive(Debug, Default)]
pub enum ForegroundedApp {
    Chrome(ForegroundedChrome),
    Finder,
    Ghostty,
    Zed,
    #[default]
    Other,
}

impl ForegroundedApp {
    /// Which app this is, dropping whatever it carries.
    #[must_use]
    pub const fn identity(&self) -> App {
        match self {
            Self::Chrome(_) => App::Chrome,
            Self::Finder => App::Finder,
            Self::Ghostty => App::Ghostty,
            Self::Zed => App::Zed,
            Self::Other => App::Other,
        }
    }

    /// The state to hold for a newly foregrounded `app`, knowing only its identity.
    #[must_use]
    pub const fn from_identity(app: App) -> Self {
        match app {
            App::Chrome => Self::Chrome(ForegroundedChrome { url: None }),
            App::Finder => Self::Finder,
            App::Ghostty => Self::Ghostty,
            App::Zed => Self::Zed,
            App::Other => Self::Other,
        }
    }
}

/// The frontmost app, and whether a navigation is in flight.
///
/// While `navigating`, `app` is the PREVIOUS app: a nav choice foregrounded a new one, but the
/// watcher has not reported it yet, so the in-app level binds nothing until it does (see
/// [`app_data`]). The fields are private; the handlers drive it through the methods below.
#[derive(Debug, Default)]
pub struct Foreground {
    app: ForegroundedApp,
    navigating: bool,
}

impl Foreground {
    /// A nav choice foregrounded an app; the watcher has not confirmed it, so `app` stays stale
    /// until it does. From the nav handlers, and undone by [`set_front_app`](Self::set_front_app).
    pub const fn start_navigating(&mut self) {
        self.navigating = true;
    }

    /// The watcher reported the front app: record it and end any pending navigation. From
    /// [`record_front_app`](crate::handlers).
    pub fn set_front_app(&mut self, app: App) {
        self.app = ForegroundedApp::from_identity(app);
        self.navigating = false;
    }

    /// The tab source reported the front tab's URL. Kept only while Chrome is the confirmed front
    /// app: a URL arriving while anything else is up describes a window nobody is looking at, and
    /// one arriving mid-navigation belongs to the app being left.
    pub fn set_tab_url(&mut self, url: String) {
        if self.navigating {
            return;
        }
        if let ForegroundedApp::Chrome(chrome) = &mut self.app {
            chrome.url = Some(url);
        }
    }

    /// The confirmed front Chrome, or `None` whenever anything else is up or a nav is in flight.
    #[must_use]
    pub const fn confirmed_chrome(&self) -> Option<&ForegroundedChrome> {
        match (&self.app, self.navigating) {
            (ForegroundedApp::Chrome(chrome), false) => Some(chrome),
            _ => None,
        }
    }

    /// The confirmed front app, or `None` while a navigation is in flight, so a key pressed in the
    /// gap does not reach the old app's bindings.
    #[must_use]
    pub const fn confirmed(&self) -> Option<App> {
        if self.navigating {
            None
        } else {
            Some(self.app.identity())
        }
    }

    /// The app the model believes is frontmost. Stale while [`navigating`](Self::navigating).
    #[must_use]
    pub const fn app(&self) -> App {
        self.app.identity()
    }

    /// Whether a nav choice is still awaiting the watcher's confirmation.
    #[must_use]
    pub const fn navigating(&self) -> bool {
        self.navigating
    }
}

#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
// This node binds nothing. `escape` leaves for home from every layer that binds keys as commands,
// but NOT from typing, where it is a key the app is waiting for, so it is bound per layer and
// typing simply does not have it. The return-home firing is bound the same way, by whichever layer
// set that timer, so it matches only its own.
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

    /// The keymap the overlay shows for this layer, read when `o` shows it.
    ///
    /// `app` is the confirmed front app, which only the in-app layer reads. Typing never binds
    /// `o`, so its arm is unreachable.
    #[must_use]
    pub const fn overlay_content(&self, app: App) -> &'static str {
        match self {
            Self::Home(_) => home::OVERLAY,
            Self::Nav(_) => nav::OVERLAY,
            Self::Resize(_) => resize::OVERLAY,
            Self::InApp(_) => app::overlay_for(app),
            Self::Typing(_) => typing::OVERLAY,
        }
    }

    /// What the status item calls this layer.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Home(_) => "Home",
            Self::Nav(_) => "Nav",
            Self::Resize(_) => "Resize",
            Self::Typing(_) => "Typing",
            Self::InApp(_) => "App",
        }
    }

    /// Reset the return-home timer of a layer whose own keys keep you in it, returning the effect
    /// that re-schedules it, or `None` for a layer that has none. Only the in-app layer qualifies:
    /// nav's and resize's keys all leave, so they keep the timer they entered with.
    #[must_use]
    fn rearm_timeout(&mut self) -> Option<MercuryEffect> {
        match self {
            Self::InApp(inapp) => Some(inapp.rearm()),
            _ => None,
        }
    }
}

/// The root's path is `&mut Self`; naming it lets the root's children say `parent = MercuryPath`.
pub type MercuryPath<'a> = &'a mut Mercury;
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;

impl Default for Mercury {
    fn default() -> Self {
        Self {
            foreground: Foreground::default(),
            typing_state: TypingState::default(),
            overlay: None,
            // Typing, the passthrough layer, so a fresh mercury (and one launched at login) leaves
            // the keyboard working rather than swallowing everything in Home. See launch-at-login.
            layer: Layer::Typing(TypingLayer::new()),
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
        let before = std::mem::discriminant(&self.layer);
        let mut effects = bind::dispatch::<MercuryStruct, Self>(self, event)?;
        // A keypress that stays in the in-app layer is activity: reset its return-home timer, so it
        // fires only after you go idle, not a fixed span after you entered.
        if matches!(event, MercuryEvent::Key(_))
            && std::mem::discriminant(&self.layer) == before
            && let Some(reset) = self.layer.rearm_timeout()
        {
            effects.push(reset);
        }
        Some(effects)
    }

    #[must_use]
    pub const fn layer(&self) -> &Layer {
        &self.layer
    }

    /// Show the active layer's keymap, or take it down if it is already up.
    ///
    /// `o` is a toggle: it is the key you press to ask what is bound, so it is the key you press
    /// again when you are done reading.
    #[must_use = "the returned effects put the overlay up or take it down"]
    pub fn toggle_overlay(&mut self) -> Vec<MercuryEffect> {
        if self.overlay.is_some() {
            return self.hide_overlay();
        }
        let content = self.layer.overlay_content(self.foreground.app());
        let (guard, effect) =
            timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
        self.overlay = Some(guard);
        vec![
            MercuryEffect::ShowOverlay(content),
            MercuryEffect::Timer(effect),
        ]
    }

    /// Take the overlay down if one is up. The dwell firing and every layer change come through
    /// here, and taking the field drops the guard, cancelling a hide that has not fired yet.
    #[must_use = "the returned effect takes the overlay off the screen"]
    pub fn hide_overlay(&mut self) -> Vec<MercuryEffect> {
        if self.overlay.take().is_some() {
            vec![MercuryEffect::HideOverlay]
        } else {
            Vec::new()
        }
    }

    /// The guard for the overlay's pending hide, which its binding matches on.
    #[must_use]
    pub const fn overlay_timer(&self) -> Option<&TimerGuard> {
        self.overlay.as_ref()
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
        self.typing_state.jk = KeySequence::new(JK, Some(JK_TIMEOUT));
        let mut effects = self.hide_overlay();
        effects.extend(match (before_passthrough, after_passthrough) {
            (true, false) => self.typing_state.held.close(),
            (false, true) => self.typing_state.held.open(),
            _ => Vec::new(),
        });
        effects.push(MercuryEffect::ShowLayer(self.layer.name()));
        effects
    }
}

/// The keys that leave typing for home.
const JK: &[Key] = &[Key::KeyJ, Key::KeyK];

/// The state the passthrough (typing) behavior needs. It lives at the root, so it outlives the
/// layer.
#[derive(Debug)]
pub struct TypingState {
    /// The physical truth about which modifier keys are down, updated by [`maybe_pass_through`] on
    /// every modifier event in every layer. It has to outlive the layer, because entering and
    /// leaving a passthrough layer reads it to synchronize the app's modifier view. See
    /// [`HeldModifiers`].
    pub held: HeldModifiers,
    /// The `jk` run. Replaced with a fresh one on every layer change, so a hold never outlives the
    /// layer it was typed in.
    pub jk: KeySequence,
}

impl Default for TypingState {
    fn default() -> Self {
        Self {
            held: HeldModifiers::default(),
            jk: KeySequence::new(JK, Some(JK_TIMEOUT)),
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
#[derive(Clone, Copy)]
pub enum Side {
    Left,
    Right,
}

impl LeftRightPair {
    #[must_use]
    pub const fn any_held(self) -> bool {
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
    /// Only the held modifiers, each with its side(s): `HeldModifiers { Meta(L,R), Alt(L) }`, or
    /// `HeldModifiers {}` when nothing is held.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeldModifiers {{")?;
        let mut any = false;
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
            write!(f, "{}{name}{sides}", if any { ", " } else { " " })?;
            any = true;
        }
        f.write_str(if any { " }" } else { "}" })
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
    pub fn open(self) -> Vec<MercuryEffect> {
        self.emit_synchronization_events(PressType::Down)
    }

    /// Leaving one: an UP for every held key, so the app forgets them.
    #[must_use]
    pub fn close(self) -> Vec<MercuryEffect> {
        self.emit_synchronization_events(PressType::Up)
    }

    /// Emit `press` for every held key, each carrying the flags as they stand after its own
    /// change, so a shared left/right bit clears only when both sides are up.
    fn emit_synchronization_events(self, press: PressType) -> Vec<MercuryEffect> {
        let mut shown = if press == PressType::Down {
            Self::default()
        } else {
            self
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
    pub const fn flags(self) -> ModifierFlags {
        let mut f = ModifierFlags::empty();
        f.set(ModifierFlags::CONTROL, self.control.any_held());
        f.set(ModifierFlags::COMMAND, self.meta.any_held());
        f.set(ModifierFlags::ALT, self.alt.any_held());
        f.set(ModifierFlags::SHIFT, self.shift.any_held());
        f
    }
}

#[must_use]
pub const fn key(key: Key) -> MercuryEvent {
    MercuryEvent::Key(KeyEvent {
        key,
        press: PressType::Down,
        flags: ModifierFlags::empty(),
    })
}

#[must_use]
pub const fn foreground(app: App) -> MercuryEvent {
    MercuryEvent::Foreground(ForegroundEvent { app })
}

/// A tab event, carrying the front tab's URL as the browser reported it.
#[must_use]
pub const fn tab(url: String) -> MercuryEvent {
    MercuryEvent::Tab(TabEvent { url })
}

/// A quit-request event (the menu bar's Quit).
#[must_use]
pub const fn quit_event() -> MercuryEvent {
    MercuryEvent::Quit(Quit)
}
