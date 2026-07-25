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

    /// Whether this is a letter key (`KeyA`..=`KeyZ`).
    #[must_use]
    pub const fn is_letter(self) -> bool {
        matches!(
            self,
            Self::KeyA
                | Self::KeyB
                | Self::KeyC
                | Self::KeyD
                | Self::KeyE
                | Self::KeyF
                | Self::KeyG
                | Self::KeyH
                | Self::KeyI
                | Self::KeyJ
                | Self::KeyK
                | Self::KeyL
                | Self::KeyM
                | Self::KeyN
                | Self::KeyO
                | Self::KeyP
                | Self::KeyQ
                | Self::KeyR
                | Self::KeyS
                | Self::KeyT
                | Self::KeyU
                | Self::KeyV
                | Self::KeyW
                | Self::KeyX
                | Self::KeyY
                | Self::KeyZ
        )
    }

    /// Whether this is a main number-row key (`Num0`..=`Num9`).
    #[must_use]
    pub const fn is_number_row(self) -> bool {
        matches!(
            self,
            Self::Num0
                | Self::Num1
                | Self::Num2
                | Self::Num3
                | Self::Num4
                | Self::Num5
                | Self::Num6
                | Self::Num7
                | Self::Num8
                | Self::Num9
        )
    }

    /// Whether this is a function key (`F1`..=`F24`).
    #[must_use]
    pub const fn is_function(self) -> bool {
        matches!(
            self,
            Self::F1
                | Self::F2
                | Self::F3
                | Self::F4
                | Self::F5
                | Self::F6
                | Self::F7
                | Self::F8
                | Self::F9
                | Self::F10
                | Self::F11
                | Self::F12
                | Self::F13
                | Self::F14
                | Self::F15
                | Self::F16
                | Self::F17
                | Self::F18
                | Self::F19
                | Self::F20
                | Self::F21
                | Self::F22
                | Self::F23
                | Self::F24
        )
    }

    /// Whether this is an arrow key.
    #[must_use]
    pub const fn is_arrow(self) -> bool {
        matches!(
            self,
            Self::UpArrow | Self::DownArrow | Self::LeftArrow | Self::RightArrow
        )
    }

    /// Whether this is a navigation key: home, end, page up/down.
    #[must_use]
    pub const fn is_navigation(self) -> bool {
        matches!(self, Self::Home | Self::End | Self::PageUp | Self::PageDown)
    }
}

/// A set of physical keys treated as one bind target.
///
/// Use like [`Key`]: `KeyGroup::Number.down().bare()`, `KeyGroup::Letter.up().with(SHIFT)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyGroup {
    /// Every key.
    Any,
    /// Letter keys: `KeyA`..=`KeyZ`.
    Letter,
    /// The main number row: `Num0`..=`Num9`.
    Number,
    /// Function keys: `F1`..=`F24`.
    Function,
    /// Modifier keys tracked as held: control, command, alt, shift (left and right).
    Modifier,
    /// Arrow keys.
    Arrow,
    /// Home, end, page up/down.
    Navigation,
}

impl KeyGroup {
    /// Whether `key` is in this group.
    #[must_use]
    pub const fn contains(self, key: Key) -> bool {
        match self {
            Self::Any => true,
            Self::Letter => key.is_letter(),
            Self::Number => key.is_number_row(),
            Self::Function => key.is_function(),
            Self::Modifier => key.is_modifier(),
            Self::Arrow => key.is_arrow(),
            Self::Navigation => key.is_navigation(),
        }
    }

    /// A trigger matching this group's keys going down.
    #[must_use]
    pub const fn down(self) -> KeyGroupPress {
        KeyGroupPress {
            group: self,
            press: PressType::Down,
        }
    }

    /// A trigger matching this group's keys coming up.
    #[must_use]
    pub const fn up(self) -> KeyGroupPress {
        KeyGroupPress {
            group: self,
            press: PressType::Up,
        }
    }
}

impl EventTrigger for KeyGroup {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.contains(event.key)
    }
}

/// A key from a [`KeyGroup`] going one direction, from [`KeyGroup::down`] or [`KeyGroup::up`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyGroupPress {
    pub group: KeyGroup,
    pub press: PressType,
}

impl EventTrigger for KeyGroupPress {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.group.contains(event.key) && event.press == self.press
    }
}

impl KeyGroupPress {
    /// Match only when exactly `flags` are held.
    #[must_use]
    pub const fn with(self, flags: ModifierFlags) -> KeyGroupChord {
        KeyGroupChord {
            group: self.group,
            press: self.press,
            flags,
        }
    }

    /// Match only when no modifier is held.
    #[must_use]
    pub const fn bare(self) -> KeyGroupChord {
        self.with(ModifierFlags::empty())
    }
}

/// A key from a [`KeyGroup`] going one direction with exactly these modifiers held.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyGroupChord {
    pub group: KeyGroup,
    pub press: PressType,
    pub flags: ModifierFlags,
}

impl EventTrigger for KeyGroupChord {
    type Event = KeyEvent;

    fn is_matching(&self, event: &KeyEvent) -> bool {
        self.group.contains(event.key) && event.press == self.press && event.flags == self.flags
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

/// A key event tagged with the consumer's device identity `D`.
///
/// `D` is whatever `intercept_with_source`'s categorize returns. The model event carries this
/// (or a newtype of it); bare [`Key`] / [`KeyPress`] still match only the key half via a
/// projecting `TryFrom` in the app.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceKeyed<E, D> {
    pub key: E,
    pub device: D,
}

/// Restricts an inner key trigger to devices the filter `D` matches.
///
/// Both halves are [`EventTrigger`]s: key policy and device policy use the same mechanism.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OnDevice<T, D> {
    pub device: D,
    pub inner: T,
}

impl<T, D> EventTrigger for OnDevice<T, D>
where
    T: EventTrigger,
    D: EventTrigger,
{
    type Event = DeviceKeyed<T::Event, D::Event>;

    fn is_matching(&self, event: &Self::Event) -> bool {
        self.device.is_matching(&event.device) && self.inner.is_matching(&event.key)
    }
}

impl<T, D> OnDevice<T, D> {
    /// Build a device-scoped trigger.
    #[must_use]
    pub const fn new(device: D, inner: T) -> Self {
        Self { device, inner }
    }
}

/// Attach a device filter to any key-side trigger.
pub trait WithDevice: Sized {
    /// Match only when the event's device satisfies `device`.
    #[must_use]
    fn on_device<D: EventTrigger>(self, device: D) -> OnDevice<Self, D> {
        OnDevice {
            device,
            inner: self,
        }
    }
}

impl<T: EventTrigger> WithDevice for T {}

#[cfg(test)]
mod tests {
    use super::{Key, KeyEvent, KeyGroup, ModifierFlags, PressType};
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
    fn key_group_categories_partition_keys() {
        assert!(KeyGroup::Letter.contains(Key::KeyA));
        assert!(!KeyGroup::Letter.contains(Key::Num1));
        assert!(KeyGroup::Number.contains(Key::Num0));
        assert!(!KeyGroup::Number.contains(Key::KeyA));
        assert!(KeyGroup::Function.contains(Key::F19));
        assert!(!KeyGroup::Function.contains(Key::Escape));
        assert!(KeyGroup::Modifier.contains(Key::ShiftLeft));
        assert!(!KeyGroup::Modifier.contains(Key::CapsLock));
        assert!(KeyGroup::Arrow.contains(Key::LeftArrow));
        assert!(KeyGroup::Navigation.contains(Key::Home));
        assert!(!KeyGroup::Navigation.contains(Key::Insert));
        assert!(KeyGroup::Any.contains(Key::Raw(0)));
    }

    #[test]
    fn key_group_number_matches_the_number_row_only() {
        let one = KeyEvent {
            key: Key::Num1,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        let a = KeyEvent {
            key: Key::KeyA,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        };
        assert!(KeyGroup::Number.is_matching(&one));
        assert!(!KeyGroup::Number.is_matching(&a));
        assert!(KeyGroup::Any.is_matching(&a));
        assert!(KeyGroup::Number.down().bare().is_matching(&one));
        assert!(
            !KeyGroup::Number
                .down()
                .with(ModifierFlags::SHIFT)
                .is_matching(&one)
        );
        let shifted = KeyEvent {
            key: Key::Num5,
            press: PressType::Down,
            flags: ModifierFlags::SHIFT,
        };
        assert!(
            KeyGroup::Number
                .down()
                .with(ModifierFlags::SHIFT)
                .is_matching(&shifted)
        );
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

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum TestDevice {
        A,
        B,
    }

    bind::self_trigger!(TestDevice);

    #[test]
    fn on_device_matches_key_and_device() {
        use super::{DeviceKeyed, WithDevice};

        let trigger = Key::KeyR.down().on_device(TestDevice::A);
        let matching = DeviceKeyed {
            key: KeyEvent {
                key: Key::KeyR,
                press: PressType::Down,
                flags: ModifierFlags::empty(),
            },
            device: TestDevice::A,
        };
        assert!(trigger.is_matching(&matching));
        assert!(!trigger.is_matching(&DeviceKeyed {
            key: KeyEvent {
                key: Key::KeyR,
                press: PressType::Down,
                flags: ModifierFlags::empty(),
            },
            device: TestDevice::B,
        }));
        assert!(!trigger.is_matching(&DeviceKeyed {
            key: KeyEvent {
                key: Key::KeyS,
                press: PressType::Down,
                flags: ModifierFlags::empty(),
            },
            device: TestDevice::A,
        }));
    }
}
