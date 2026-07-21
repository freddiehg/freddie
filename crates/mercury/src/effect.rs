//! What a handler asks the consumer to do, and the window placement it can request.

use freddie::TimerEffect;
use freddie_keys::{Key, KeyEvent, KeyPress, ModifierFlags, PressType};

use freddie_windows::WindowFrame;

use crate::MercuryEvent;

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

/// Which part of a URL a copy puts on the clipboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlPart {
    /// The URL as the browser reports it.
    Whole,
    /// The host it names, `www.` and all: `https://www.x.com/asdfasdf` copies `www.x.com`.
    Host,
}

/// The text a copy puts on the clipboard, and where it comes from.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum Copied {
    /// Text mercury already holds. The extension reports the front tab's URL as it changes, so
    /// this is the usual case for a copy, and it costs a string.
    Text(String),
    /// The front Chrome tab's URL, read back out of Chrome, and the part of it to keep.
    ///
    /// The fallback for when nothing reported one: no extension connected, or a page it never
    /// sees. It asks the app rather than the model, so it is a subprocess and an Apple Events
    /// permission, which is why it is not the way this normally works.
    FrontTabUrl(UrlPart),
}

/// What a handler asks the consumer to do. Inert data; performing it is the consumer's job,
/// and it never mutates Mercury's state directly.
// Effect equality is only ever asked for by the tests (dispatch never compares effects), so the
// derive is gated behind `testing` and kept out of the normal build. `PartialEq` but not `Eq`,
// because a timer carries a `MercuryEvent`, and a window event in one carries `f64` frames.
#[cfg_attr(feature = "testing", derive(PartialEq))]
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
    /// Move and resize one window, named by id, to a rectangle already worked out.
    ///
    /// The sink does not ask what is frontmost, what is focused, or what the screen looks
    /// like. The handler that produced this read all of it out of the model.
    SetFrame(WindowFrame),
    /// Put text on the clipboard, replacing what is there.
    Copy(Copied),
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

/// One effect iterates as itself, so a handler with a single thing to ask for returns it bare
/// and dispatch collects it. `Bindings::Output` stays the vector, since a handler that asks for
/// several is the other half of the same trait.
impl IntoIterator for MercuryEffect {
    type Item = Self;
    type IntoIter = std::iter::Once<Self>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self)
    }
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
