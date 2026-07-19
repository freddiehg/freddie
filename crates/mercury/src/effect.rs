//! What a handler asks the consumer to do, and the window placement it can request.

use freddie::TimerEffect;
use freddie_keys::{Key, KeyEvent, KeyPress, ModifierFlags, PressType};

use crate::MercuryEvent;

/// Where a window should go. Mercury's own, mirroring `freddie_windows::Placement` so the
/// model stays free of the OS crates, the way `App` is free of bundle ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Maximize,
    LeftHalf,
    RightHalf,
}

/// One key carrying its modifiers as flags, which is how Mercury spells a chord.
///
/// `cmd`-`r` is `Chord { key: KeyR, flags: COMMAND }`: one key event with the modifier as a flag,
/// not a synthetic `cmd` down and up around it. A synthetic modifier event would strand the
/// modifier the user is really holding, because the app counts the extra up and thinks it released.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Clone, Copy, Debug)]
pub struct Chord {
    pub key: Key,
    pub flags: ModifierFlags,
}

/// What a handler asks the consumer to do. Inert data; performing it is the consumer's job,
/// and it never mutates Mercury's state directly.
// Effect equality is only ever asked for by the tests (dispatch never compares effects), so the
// derive is gated behind `testing` and kept out of the normal build.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum MercuryEffect {
    Foreground(super::App),
    /// Tap one key with modifiers baked into both halves.
    Tap(Chord),
    /// Emit one raw key event, a press or a release on its own.
    ///
    /// The escape hatch, for the one case that is genuinely a lone half of a keypress: passing
    /// a key through, where the model sees a down and an up as separate events and re-emits
    /// each. Building a chord out of these is a bug waiting to happen; use [`Tap`](Self::Tap).
    Emit(KeyEvent),
    /// Move and resize the focused window of the frontmost app.
    Place(Placement),
    /// Quit the program. The effect handler performs this by exiting.
    Kill,
    /// Put the overlay up, showing `text`. Replaces whatever it was showing.
    ShowOverlay(&'static str),
    /// Take the overlay down. A no-op if nothing is up.
    HideOverlay,
    /// Show this layer's name in the menu bar. Produced only by `set_layer`, so the item and the
    /// model cannot disagree about which layer is active.
    ShowLayer(&'static str),
    /// Arm a timer. The effect loop schedules it; it fires its event after the delay unless the
    /// guard held by the state that asked for it drops first.
    Timer(TimerEffect<MercuryEvent>),
}

pub(crate) const fn tap(key: Key, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Tap(Chord { key, flags })
}

/// Emit one key event carrying `flags`. The building block for passing a key through and for the
/// modifier synchronization sweeps.
pub(crate) const fn emit(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}

/// The effects that replay what a key sequence swallowed. Every key a run takes arrived with no
/// modifier, so every one of them goes back out bare.
pub(crate) fn replay(presses: Vec<KeyPress>) -> Vec<MercuryEffect> {
    presses
        .into_iter()
        .map(|p| emit(p.key, p.press, ModifierFlags::empty()))
        .collect()
}
