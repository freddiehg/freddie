//! The platform-neutral keyboard vocabulary shared across freddie.
//!
//! [`Key`] names physical keys independent of any OS. It is the type consumers
//! bind against, and the type each `freddie_keyboard` backend maps its native key
//! codes to and from. Because this crate owns the type, [`Key`] is a `bind`
//! trigger directly, so a binding reads `Key::KeyR` with no wrapper.
//!
//! The named variants are exhaustive on purpose, so a backend's keycode table is a
//! `match` and a missing mapping is a compile error. [`Key::Raw`] carries a native
//! code with no name, both for keys the table lacks and for made-up keys.

use bind::{EventTrigger, Match};

/// A physical key, named by its US-ANSI position, independent of layout or OS.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
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

    /// A native key code with no name: a key the table does not cover, or a
    /// made-up key used as a remap intermediary. Not portable across OSes.
    Raw(u16),
}

/// Whether a key went down or came up.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PressType {
    Down,
    Up,
}

/// A key going down or coming up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
}

/// The priority a specific key binds at. A wildcard (a catch-all key) should bind
/// below this, so a named key wins wherever the two overlap.
pub const SPECIFIC: bind::Priority = 0;

impl EventTrigger for Key {
    type Event = KeyEvent;

    fn try_match(&self, event: &KeyEvent) -> Match {
        if *self == event.key {
            Match::Handle(SPECIFIC)
        } else {
            Match::DontHandle
        }
    }
}

impl Key {
    /// A trigger matching only this key's press.
    #[must_use]
    pub const fn down(self) -> KeyPress {
        KeyPress {
            key: self,
            press: PressType::Down,
        }
    }

    /// A trigger matching only this key's release.
    #[must_use]
    pub const fn up(self) -> KeyPress {
        KeyPress {
            key: self,
            press: PressType::Up,
        }
    }
}

/// A trigger matching a key going one direction, from [`Key::down`] or [`Key::up`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyPress {
    pub key: Key,
    pub press: PressType,
}

impl EventTrigger for KeyPress {
    type Event = KeyEvent;

    fn try_match(&self, event: &KeyEvent) -> Match {
        if self.key == event.key && self.press == event.press {
            Match::Handle(SPECIFIC)
        } else {
            Match::DontHandle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, KeyEvent, PressType, SPECIFIC};
    use bind::{EventTrigger, Match};

    #[test]
    fn matches_only_its_own_key() {
        let event = KeyEvent {
            key: Key::KeyR,
            press: PressType::Down,
        };
        assert_eq!(Key::KeyR.try_match(&event), Match::Handle(SPECIFIC));
        assert_eq!(Key::KeyS.try_match(&event), Match::DontHandle);
    }

    #[test]
    fn raw_matches_by_code() {
        let event = KeyEvent {
            key: Key::Raw(64000),
            press: PressType::Down,
        };
        assert_eq!(Key::Raw(64000).try_match(&event), Match::Handle(SPECIFIC));
        assert_eq!(Key::Raw(1).try_match(&event), Match::DontHandle);
        assert_eq!(Key::KeyA.try_match(&event), Match::DontHandle);
    }
}
