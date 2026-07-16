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
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MercuryEffect {
    /// Bring an app to the foreground.
    Foreground(super::App),
    /// Tap `key` while `modifiers` are held. The chord.
    ///
    /// The modifiers are pressed, the key is tapped, and they are released, so a key cannot
    /// carry a modifier that was never pressed. Prefer this to a hand-written sequence of
    /// [`Emit`](Self::Emit)s.
    Tap { modifiers: Vec<Key>, key: Key },
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

/// Tap `key` while `modifiers` are held.
pub(crate) fn tap(modifiers: &[Key], key: Key) -> MercuryEffect {
    MercuryEffect::Tap {
        modifiers: modifiers.to_vec(),
        key,
    }
}

/// Emit one key event carrying `flags`. The building block for passing a key through and for the
/// modifier synchronization sweeps.
pub(crate) fn emit(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}
