//! What a handler asks the consumer to do, and the window placement it can request.

use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

/// Where a window should go. Mercury's own, mirroring `freddie_windows::Placement` so the
/// model stays free of the OS crates, the way `App` is free of bundle ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Maximize,
    LeftHalf,
    RightHalf,
}

/// What a handler asks the consumer to do. Inert data; performing it is the consumer's job,
/// and it never mutates Mercury's state directly.
// Effect equality is only ever asked for by the tests (dispatch never compares effects), so the
// derive is gated behind `testing` and kept out of the normal build.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum MercuryEffect {
    Foreground(super::App),
    /// Tap `key` with `flags` baked into both halves. The chord.
    ///
    /// `cmd`-`r` is `Tap { key: KeyR, flags: COMMAND }`: one key carrying the modifier as a flag,
    /// not a synthetic `cmd` down and up around it. A synthetic modifier event would strand the
    /// modifier the user is really holding (the app counts the extra up and thinks it released).
    Tap {
        key: Key,
        flags: ModifierFlags,
    },
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
}

pub(crate) const fn tap(key: Key, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Tap { key, flags }
}

/// Emit one key event carrying `flags`. The building block for passing a key through and for the
/// modifier synchronization sweeps.
pub(crate) const fn emit(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}
