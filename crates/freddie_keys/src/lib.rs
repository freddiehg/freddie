//! The platform-neutral keyboard vocabulary shared across freddie.
//!
//! [`Keyboard`] names physical keys independent of any OS. It is the type
//! consumers bind against, and the type each `freddie_keyboard` backend maps its
//! native key codes to and from. Because this crate owns the type, [`Keyboard`]
//! is a `bind` trigger directly, so a binding reads `Keyboard::KeyR` with no
//! wrapper.
//!
//! The enum is exhaustive on purpose: a backend's keycode table is a `match` over
//! it, so a missing mapping is a compile error rather than a silent gap.

use bind::EventTrigger;

/// A physical key, named by its US-ANSI position, independent of layout or OS.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Keyboard {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,

    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,

    Escape,
    Return,
    Space,
    Tab,
    Backspace,
    Delete,
    CapsLock,

    UpArrow,
    DownArrow,
    LeftArrow,
    RightArrow,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,

    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    MetaLeft,
    MetaRight,

    Grave,
    Minus,
    Equal,
    LeftBracket,
    RightBracket,
    BackSlash,
    SemiColon,
    Quote,
    Comma,
    Dot,
    Slash,
}

/// A key going down or coming up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    /// Which key.
    pub key: Keyboard,
    /// `true` on key-down, `false` on key-up.
    pub down: bool,
}

impl EventTrigger for Keyboard {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        *self == event.key
    }
}

#[cfg(test)]
mod tests {
    use super::{Keyboard, KeyEvent};
    use bind::EventTrigger;

    #[test]
    fn matches_only_its_own_key() {
        let event = KeyEvent {
            key: Keyboard::KeyR,
            down: true,
        };
        assert!(Keyboard::KeyR.is_matching(&event));
        assert!(!Keyboard::KeyS.is_matching(&event));
    }
}
