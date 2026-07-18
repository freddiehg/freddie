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

use bind::EventTrigger;

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

/// A key going down or coming up, carrying its modifier flags.
///
/// The flags are authoritative: the source stamps them at creation (macOS from the hardware
/// modifier state for a physical key, the posting app for an injected one). A passed-through key
/// carries exactly these; a sync sweep or a chord builds its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags,
}

/// The modifier keys an emitted event carries, as a portable bitset. `freddie_keyboard` maps it
/// to the platform's native flags when it posts the event.
///
/// A `CGEvent`'s own flags are baked in from the source's state when it is created, which lags a
/// modifier posted microseconds earlier, so a chord posted back to back carries the wrong flags.
/// Stating the flags on the event and applying exactly them makes the emitted stream say what it
/// means, whatever the source thinks.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ModifierFlags(u8);

impl ModifierFlags {
    pub const CONTROL: Self = Self(1 << 0);
    pub const COMMAND: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);
    /// The `fn` (Globe) modifier. Not a key tracked as held (it arrives only as a flag on other
    /// events), so it rides through solely on this bit.
    pub const FN: Self = Self(1 << 4);

    /// No modifiers.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bits, for a backend mapping them to native flags.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every bit in `flag` is set.
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    /// Set or clear `flag`.
    pub const fn set(&mut self, flag: Self, on: bool) {
        self.0 = if on {
            self.0 | flag.0
        } else {
            self.0 & !flag.0
        };
    }
}

impl EventTrigger for Key {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        *self == event.key
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

    /// Whether this is a modifier key tracked as held: control, command, alt, or shift, left or
    /// right. Caps lock (a lock) and fn (no variant) are not modifiers here.
    #[must_use]
    pub const fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::ControlLeft
                | Self::ControlRight
                | Self::MetaLeft
                | Self::MetaRight
                | Self::AltLeft
                | Self::AltRight
                | Self::ShiftLeft
                | Self::ShiftRight
        )
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

    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.key == event.key && self.press == event.press
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, KeyEvent, ModifierFlags, PressType};
    use bind::EventTrigger;

    #[test]
    fn matches_only_its_own_key() {
        let event = KeyEvent {
            key: Key::KeyR,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        assert!(Key::KeyR.is_matching(&event));
        assert!(!Key::KeyS.is_matching(&event));
    }

    #[test]
    fn raw_matches_by_code() {
        let event = KeyEvent {
            key: Key::Raw(64000),
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        assert!(Key::Raw(64000).is_matching(&event));
        assert!(!Key::Raw(1).is_matching(&event));
        assert!(!Key::KeyA.is_matching(&event));
    }
}
