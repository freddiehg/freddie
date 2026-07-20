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
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModifierFlags(u8);

impl std::fmt::Debug for KeyEvent {
    /// `KeyEvent { key: KeyJ, press: Down }`, with `flags` only when some modifier is set.
    ///
    /// Every dispatched event goes in the log, and most keys carry no modifier, so the derive
    /// spent a third of each line saying so.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KeyEvent {{ key: {:?}, press: {:?}",
            self.key, self.press
        )?;
        if !self.flags.is_empty() {
            write!(f, ", flags: {:?}", self.flags)?;
        }
        f.write_str(" }")
    }
}

impl std::fmt::Debug for ModifierFlags {
    /// The modifiers set, by name: `ModifierFlags(COMMAND|SHIFT)`, or `ModifierFlags()` for none.
    /// The derive printed the raw bits, which nothing can read at a glance.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModifierFlags(")?;
        let mut any = false;
        for (name, flag) in [
            ("CONTROL", Self::CONTROL),
            ("COMMAND", Self::COMMAND),
            ("ALT", Self::ALT),
            ("SHIFT", Self::SHIFT),
            ("FN", Self::FN),
        ] {
            if self.contains(flag) {
                if any {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                any = true;
            }
        }
        f.write_str(")")
    }
}

impl std::ops::BitOr for ModifierFlags {
    type Output = Self;

    /// The union of two sets, so a chord's modifiers read as `COMMAND | SHIFT`.
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

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

impl KeyPress {
    /// A trigger matching this press only when exactly `flags` are held.
    #[must_use]
    pub const fn with(self, flags: ModifierFlags) -> KeyChord {
        KeyChord {
            key: self.key,
            press: self.press,
            flags,
        }
    }

    /// A trigger matching this press only when no modifier is held.
    ///
    /// The counterpart to [`with`](Self::with): a node that binds one key at several modifier
    /// combinations spells every one of them as a chord, so no two of its triggers can match the
    /// same event and which one wins is not a question about declaration order.
    #[must_use]
    pub const fn bare(self) -> KeyChord {
        self.with(ModifierFlags::empty())
    }
}

/// A key going one direction with exactly these modifiers held, from [`KeyPress::with`].
///
/// Where [`KeyPress`] ignores the flags an event carries, this matches them exactly, so `cmd`-`l`
/// and a bare `l` are different triggers. Caps lock is not a [`ModifierFlags`] bit (the backend
/// leaves `AlphaShift` out of its mapping), so a chord matches with caps lock on or off.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyChord {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags,
}

impl EventTrigger for KeyChord {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.key == event.key && self.press == event.press && self.flags == event.flags
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
    fn debug_leaves_out_flags_when_there_are_none() {
        // Every dispatched event is logged, and most keys carry no modifier.
        let bare = KeyEvent {
            key: Key::KeyJ,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        assert_eq!(format!("{bare:?}"), "KeyEvent { key: KeyJ, press: Down }");

        let mut flags = ModifierFlags::COMMAND;
        flags.set(ModifierFlags::SHIFT, true);
        let chord = KeyEvent {
            key: Key::KeyV,
            press: PressType::Up,
            flags,
        };
        assert_eq!(
            format!("{chord:?}"),
            "KeyEvent { key: KeyV, press: Up, flags: ModifierFlags(COMMAND|SHIFT) }"
        );
    }

    #[test]
    fn a_chord_matches_only_its_own_modifiers() {
        let bare = KeyEvent {
            key: Key::KeyL,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        let with_command = KeyEvent {
            key: Key::KeyL,
            press: PressType::Down,
            flags: ModifierFlags::COMMAND,
        };
        let with_both = KeyEvent {
            key: Key::KeyL,
            press: PressType::Down,
            flags: ModifierFlags::COMMAND | ModifierFlags::SHIFT,
        };

        // The three are mutually exclusive: each event matches exactly one of them.
        for (trigger, matching) in [
            (Key::KeyL.down().bare(), &bare),
            (Key::KeyL.down().with(ModifierFlags::COMMAND), &with_command),
            (
                Key::KeyL
                    .down()
                    .with(ModifierFlags::COMMAND | ModifierFlags::SHIFT),
                &with_both,
            ),
        ] {
            for event in [&bare, &with_command, &with_both] {
                assert_eq!(
                    trigger.is_matching(event),
                    std::ptr::eq(event, matching),
                    "{trigger:?} against {event:?}"
                );
            }
        }
    }

    #[test]
    fn a_chord_matches_neither_the_other_key_nor_the_release() {
        let trigger = Key::KeyL.down().with(ModifierFlags::COMMAND);
        assert!(!trigger.is_matching(&KeyEvent {
            key: Key::KeyK,
            press: PressType::Down,
            flags: ModifierFlags::COMMAND,
        }));
        assert!(!trigger.is_matching(&KeyEvent {
            key: Key::KeyL,
            press: PressType::Up,
            flags: ModifierFlags::COMMAND,
        }));
    }

    // A plain press ignores the flags, which is why a node binding one key at several modifier
    // combinations has to spell every one of them as a chord.
    #[test]
    fn a_press_matches_whatever_modifiers_are_held() {
        let trigger = Key::KeyL.down();
        assert!(trigger.is_matching(&KeyEvent {
            key: Key::KeyL,
            press: PressType::Down,
            flags: ModifierFlags::COMMAND,
        }));
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
