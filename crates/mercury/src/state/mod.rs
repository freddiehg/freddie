//! The state tree: the nodes, their bindings, and the path aliases that chain them.
//!
//! The `#[bind(.. => handler)]` attributes name handlers that live in [`crate::handlers`], so
//! this module glob-imports them: the derive generates a call to each named handler here, at
//! the node's definition site.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use bind::{Bind, if_not_invalidated};
use freddie::{TimerFired, TimerGuard, timer_effect_and_guard};
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};
use freddie_windows::{Frame, Monitor, Snapshot, WindowChange, WindowFrame, WindowId};
use laserbeam::PathMut;

// The derive generates a call to each named handler at its node's definition site below, so
// every handler has to be in scope here. A glob keeps this in step with the handler set instead
// of a name-by-name list that drifts.
use crate::effect::emit;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{
    AnyKey, App, ForegroundEvent, Foregrounded, MercuryEffect, MercuryEvent, MercuryStruct, Quit,
    Site, TabEvent, Tabbed, Windowed,
};

mod app;
mod home;
mod nav;
mod resize;
mod return_home;
mod site;
mod typing;

pub use app::{AppData, AppLayer, ChromeApp, GhosttyApp};
pub use home::HomeLayer;
pub use nav::NavLayer;
pub use resize::ResizeLayer;
pub use return_home::{AndReturnHome, ReturnHomeLayers};
pub use site::{ClaudeAiSite, SiteData, SiteLayer};
pub(crate) use typing::arm_jk_timeout;
pub use typing::{JK_TIMEOUT, TypingLayer};

/// How long a chooser layer sits idle before returning home.
pub const RETURN_TO_HOME_TIMEOUT: Duration = Duration::from_secs(10);

/// Arm the return-to-home timer a layer holds: the guard cancels it on drop, and the effect
/// schedules it. It fires after [`RETURN_TO_HOME_TIMEOUT`], and the layer that set it binds that
/// firing home, matching on the guard it still holds.
pub(crate) fn arm_return_home() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, |id| {
        MercuryEvent::Timer(TimerFired(id))
    });
    (guard, MercuryEffect::Timer(effect))
}

/// How long the overlay stays up before its hide timer fires.
pub const OVERLAY_DWELL: Duration = Duration::from_secs(10);

#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => if_not_invalidated(record_front_app),
    Tabbed => if_not_invalidated(record_tab_url),
    Windowed => if_not_invalidated(record_windows),
    Quit => if_not_invalidated(quit),
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => if_not_invalidated(hide_overlay),
    |mercury_path| mercury_path.windows.pending_timer().map(TimerGuard::trigger) => if_not_invalidated(placement_settled),
)]
// `o` and escape bind once, here: in typing, its catch-all claims both keys before the root's
// rows run, so an `o` is an `o` and an escape is the app's.
#[bind(
    Key::KeyO.down() => if_not_invalidated(toggle_overlay),
    Key::Escape.down() => if_not_invalidated(go_home),
)]
#[post(AnyKey => track_held_modifiers)]
pub struct Mercury {
    /// The watcher-confirmed frontmost app, or `None` while a nav choice's `Foreground` effect is
    /// in flight: the choice sets it to `None`, and the watcher's report sets it back to `Some`,
    /// so nothing binds against the app being left in the gap.
    pub foreground: Option<ForegroundedApp>,
    /// Every window mercury knows about, and the monitors they sit on. See [`Windows`].
    pub windows: Windows,
    /// The physical truth about which modifier keys are down, kept current by the
    /// `track_held_modifiers` post on every key in every layer. Entering and leaving typing
    /// reads it to synchronize the app's modifier view. See [`HeldModifiers`].
    pub held: HeldModifiers,
    /// The overlay currently up, if any: the guard for its pending hide. The overlay is an
    /// external window driven by effects, so this is its only trace in the model, held at the root
    /// because there is one overlay across all layers.
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

    /// The Chrome state, if this is Chrome.
    #[must_use]
    pub const fn chrome(&self) -> Option<&ForegroundedChrome> {
        match self {
            Self::Chrome(chrome) => Some(chrome),
            _ => None,
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

/// What mercury knows about the windows on screen.
///
/// Filled entirely by the window source: a snapshot at startup and a change per event after
/// it. Handlers read it and never read the OS, so what a placement computes is a function of
/// state and event like everything else.
#[derive(Default)]
pub struct Windows {
    /// Every open window: where it is, and where it goes back to.
    open: HashMap<WindowId, WindowState>,
    /// The focused window, `None` when nothing is focused or its id is unreadable.
    focused: Option<WindowId>,
    /// The monitors, in the order the source reported them.
    screens: Vec<Monitor>,
    /// The placement mercury has asked for and not yet seen land. See [`PendingPlacement`].
    pending: Option<PendingPlacement>,
}

/// Every dispatched event logs the whole state on one line, and the derived `Debug` for this puts
/// every open window, its frame, and its restore frame on that line. That is a hundred numbers
/// nobody reads, and it buries the fields of the record that are the point of it.
///
/// So this prints its name and nothing else. What a window is doing is already in the log: the
/// window source logs each change as it arrives, and a placement logs the frame it asked for.
impl fmt::Debug for Windows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Windows")
    }
}

/// One window: where it is now, and where a restore would put it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct WindowState {
    /// Where the window is, as the source last reported it.
    frame: Frame,
    /// Where the window was before mercury first moved it. `None` once it is back there, or
    /// once the user has moved it since.
    restore: Option<Frame>,
}

/// A [`MercuryEffect::SetFrame`] that has been asked for and not yet landed.
///
/// While one is outstanding, every move reported for its window is mercury's own doing, so
/// the remembered frame survives it. One placement produces several reports, and only the
/// last is the frame that was asked for.
#[derive(Debug)]
struct PendingPlacement {
    window: WindowId,
    /// Held for its `Drop` and for the trigger that matches its firing: the wait ends when
    /// this fires, and only then.
    timer: TimerGuard,
}

/// How long a placement has to land before the window is the user's again.
///
/// It bounds how long a drag can be mistaken for mercury's own placement, so shorter is
/// better, but it has to cover two position-and-size writes and the reports they produce.
pub const PLACEMENT_SETTLE: Duration = Duration::from_millis(250);

impl Windows {
    /// The state the window source found when it started watching, before any change.
    #[must_use]
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            open: snapshot
                .windows
                .into_iter()
                .map(|w| {
                    (
                        w.window,
                        WindowState {
                            frame: w.frame,
                            restore: None,
                        },
                    )
                })
                .collect(),
            focused: snapshot.focused,
            screens: snapshot.screens,
            pending: None,
        }
    }

    /// The focused window and its frame, which is what every placement starts from.
    ///
    /// `None` when nothing is focused, or when the focused window has no frame on record:
    /// a focus report can name a window no `Opened` ever did.
    #[must_use]
    pub fn focused(&self) -> Option<WindowFrame> {
        let window = self.focused?;
        Some(WindowFrame {
            window,
            frame: self.open.get(&window)?.frame,
        })
    }

    /// The monitor a frame's top-left corner is on, or the first monitor if it is on none.
    /// `None` only before the first `Screens` report.
    #[must_use]
    pub fn monitor_for(&self, frame: Frame) -> Option<Monitor> {
        self.screens
            .iter()
            .find(|m| m.full.contains(frame.x, frame.y))
            .or_else(|| self.screens.first())
            .copied()
    }

    /// Apply one reported change.
    ///
    /// Every arm assigns, replaces, or removes. None accumulates, so applying a change twice
    /// lands where applying it once does, which is what makes the boot ordering safe: see the
    /// idempotence rule in `CLAUDE.md`.
    pub(crate) fn record(&mut self, change: &WindowChange) {
        match change {
            WindowChange::Opened(w) => {
                self.open.insert(
                    w.window,
                    WindowState {
                        frame: w.frame,
                        restore: None,
                    },
                );
            }
            WindowChange::Moved(w) | WindowChange::Resized(w) => {
                let ours = self.pending_covers(*w);
                if let Some(state) = self.open.get_mut(&w.window) {
                    state.frame = w.frame;
                    if !ours {
                        state.restore = None;
                    }
                }
            }
            WindowChange::Closed(window) => {
                self.open.remove(window);
                if self.focused == Some(*window) {
                    self.focused = None;
                }
            }
            WindowChange::Focused(window) => self.focused = *window,
            WindowChange::Screens(screens) => self.screens.clone_from(screens),
        }
    }

    /// Whether `moved` is a report of mercury's own outstanding placement.
    ///
    /// Every report for the pending window counts, and the wait ends on the timer rather
    /// than on the frame that was asked for. One placement writes the position and the size
    /// twice, so the frame asked for is reported more than once; ending the wait on the
    /// first of them would leave the rest looking like the user dragging the window, and
    /// they would forget the frame the restore needs.
    fn pending_covers(&self, moved: WindowFrame) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.window == moved.window)
    }

    /// The guard for the placement still outstanding, for the trigger that matches its
    /// firing.
    pub(crate) fn pending_timer(&self) -> Option<&TimerGuard> {
        self.pending.as_ref().map(|p| &p.timer)
    }

    /// The placement mercury asked for has had its time: what the window does next is the
    /// user's.
    pub(crate) fn forget_pending(&mut self) {
        self.pending = None;
    }

    /// Ask for `target`, and wait for it to land.
    ///
    /// The wait is what keeps the moves this causes from counting as the user's. Both
    /// callers need it; what they differ on is the remembered frame, which this leaves
    /// alone.
    fn asking_for(&mut self, target: WindowFrame) -> Vec<MercuryEffect> {
        let (timer, effect) =
            timer_effect_and_guard(PLACEMENT_SETTLE, |id| MercuryEvent::Timer(TimerFired(id)));
        self.pending = Some(PendingPlacement {
            window: target.window,
            timer,
        });
        vec![
            MercuryEffect::SetFrame(target),
            MercuryEffect::Timer(effect),
        ]
    }

    /// Remember where the window is now, then place it.
    ///
    /// The frame it has now becomes the one a restore goes back to, unless one is already
    /// remembered: a run of placements all restore to where the window was before the first
    /// of them.
    pub(crate) fn placing(&mut self, target: WindowFrame) -> Vec<MercuryEffect> {
        let Some(state) = self.open.get_mut(&target.window) else {
            return Vec::new();
        };
        let frame = state.frame;
        state.restore.get_or_insert(frame);
        self.asking_for(target)
    }

    /// Take the focused window's remembered frame, and return the effects that put it back.
    ///
    /// Empty when nothing is focused or the window has no remembered frame: nothing placed
    /// it, or it is already back. Taking, not reading: a restored window is where it
    /// started, so there is nothing left to put it back to.
    pub(crate) fn restoring(&mut self) -> Vec<MercuryEffect> {
        let Some(window) = self.focused else {
            return Vec::new();
        };
        let Some(frame) = self
            .open
            .get_mut(&window)
            .and_then(|state| state.restore.take())
        else {
            return Vec::new();
        };
        self.asking_for(WindowFrame { window, frame })
    }
}

#[derive(Bind, Debug, derive_more::From)]
#[node(parent_path = MercuryPath)]
#[binds(MercuryStruct)]
pub enum Layer {
    Home(HomeLayer),
    Typing(TypingLayer),
    ReturnHome(AndReturnHome<ReturnHomeLayers>),
}

impl Layer {
    /// A passthrough layer hands every key to the app, so entering and leaving one synchronizes
    /// the app's view of the held modifiers (see [`Mercury::set_layer`]). Typing is the only one;
    /// add more by returning true for them.
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        matches!(self, Self::Typing(_))
    }

    /// The keymap the overlay shows for this layer, read when `o` shows it.
    ///
    /// `foreground` is the confirmed front app, which only the in-app and site arms read; while a
    /// nav is in flight they fall back to their generic cards. Typing never binds `o`, so its arm
    /// is unreachable.
    #[must_use]
    pub fn overlay_content(&self, foreground: Option<&ForegroundedApp>) -> &'static str {
        match self {
            Self::Home(_) => home::OVERLAY,
            Self::Typing(_) => typing::OVERLAY,
            Self::ReturnHome(w) => match w.layers() {
                ReturnHomeLayers::Nav(_) => nav::OVERLAY,
                ReturnHomeLayers::Resize(_) => resize::OVERLAY,
                ReturnHomeLayers::InApp(_) => {
                    app::overlay_for(foreground.map_or(App::Other, ForegroundedApp::identity))
                }
                // The site layer's keymap is the front tab's, so it needs the URL, not just the app.
                ReturnHomeLayers::Site(_) => site::overlay_for(
                    foreground
                        .and_then(ForegroundedApp::chrome)
                        .and_then(|chrome| chrome.url.as_deref())
                        .map(Site::from_url),
                ),
            },
        }
    }

    /// What the status item calls this layer.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Home(_) => "Home",
            Self::Typing(_) => "Typing",
            Self::ReturnHome(w) => match w.layers() {
                ReturnHomeLayers::Nav(_) => "Nav",
                ReturnHomeLayers::Resize(_) => "Resize",
                ReturnHomeLayers::InApp(_) => "App",
                ReturnHomeLayers::Site(_) => "Site",
            },
        }
    }
}

/// The root's path is `&mut Self`; naming it lets the root's children say `parent = MercuryPath`.
pub type MercuryPath<'a> = &'a mut Mercury;
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type TypingLayerPath<'a> = PathMut<TypingLayer, LayerPath<'a>>;
pub type AndReturnHomePath<'a> = PathMut<AndReturnHome<ReturnHomeLayers>, LayerPath<'a>>;
pub type ReturnHomeLayersPath<'a> = PathMut<ReturnHomeLayers, AndReturnHomePath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, ReturnHomeLayersPath<'a>>;
pub type SiteLayerPath<'a> = PathMut<SiteLayer, ReturnHomeLayersPath<'a>>;

impl Mercury {
    /// The layer a fresh mercury boots into: Typing, the passthrough layer, so a fresh mercury
    /// (and one launched at login) leaves the keyboard working rather than swallowing everything
    /// in Home. See launch-at-login.
    fn boot_layer() -> Layer {
        Layer::Typing(TypingLayer::new())
    }

    /// The title the status item shows before the first layer change.
    ///
    /// The main thread paints this when it creates the status item, before the model that would
    /// otherwise hand it over exists. A literal rather than `boot_layer().name()`, which is not a
    /// const expression; `boot_title_matches_the_boot_layer` keeps the two from drifting.
    pub const BOOT_TITLE: &'static str = "Typing";

    /// The model at boot, told what the sources already know.
    ///
    /// `front_app` is read before the main loop runs; see `refactors/past/seed-at-construction.md`.
    /// No `Default`, because a `Mercury` that has not been told what is frontmost would
    /// resolve its in-app layer against the wrong app until something corrected it.
    #[must_use]
    pub fn new(front_app: App, windows: Windows) -> Self {
        Self {
            foreground: Some(ForegroundedApp::from_identity(front_app)),
            windows,
            held: HeldModifiers::default(),
            overlay: None,
            layer: Self::boot_layer(),
        }
    }

    /// A fresh Mercury with `layer` active, and no particular app frontmost. For tests; a live
    /// transition goes through [`set_layer`](Self::set_layer).
    #[must_use]
    pub fn with_layer(layer: Layer) -> Self {
        Self {
            layer,
            ..Self::new(App::Other, Windows::default())
        }
    }

    /// Dispatches one event, returning the effects it produced, which are the caller's to
    /// perform.
    #[must_use]
    pub fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> {
        bind::dispatch::<MercuryStruct, Self, _>(self, event)
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
        let content = self.layer.overlay_content(self.foreground.as_ref());
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
        let mut effects = self.hide_overlay();
        effects.extend(match (before_passthrough, after_passthrough) {
            (true, false) => self.held.close(),
            (false, true) => self.held.open(),
            _ => Vec::new(),
        });
        effects.push(MercuryEffect::ShowLayer(self.layer.name()));
        effects
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
