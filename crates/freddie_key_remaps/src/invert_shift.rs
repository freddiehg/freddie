use freddie_keys::{KeyEvent, ModifierFlags};

/// Toggle the SHIFT bit on a key event; other modifiers are left alone.
///
/// Bare becomes SHIFT, SHIFT becomes bare; `CONTROL` alone becomes `CONTROL|SHIFT`, and so on.
/// Physical key and press type are unchanged.
///
/// Used for number-row invert (`1` ↔ `!`), backslash ↔ pipe, and similar.
#[must_use]
pub const fn invert_shift(ev: &KeyEvent) -> KeyEvent {
    let mut flags = ev.flags;
    let shift_on = !flags.contains(ModifierFlags::SHIFT);
    flags.set(ModifierFlags::SHIFT, shift_on);
    KeyEvent {
        key: ev.key,
        press: ev.press,
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freddie_keys::{Key, PressType};

    fn ev(key: Key, flags: ModifierFlags) -> KeyEvent {
        KeyEvent {
            key,
            press: PressType::Down,
            flags,
        }
    }

    #[test]
    fn bare_gains_shift() {
        let got = invert_shift(&ev(Key::Num1, ModifierFlags::empty()));
        assert!(got.flags.contains(ModifierFlags::SHIFT));
    }

    #[test]
    fn shift_only_drops_shift() {
        let got = invert_shift(&ev(Key::Num1, ModifierFlags::SHIFT));
        assert!(got.flags.is_empty());
    }

    #[test]
    fn preserves_other_modifiers() {
        let got = invert_shift(&ev(
            Key::BackSlash,
            ModifierFlags::CONTROL | ModifierFlags::SHIFT,
        ));
        assert!(got.flags.contains(ModifierFlags::CONTROL));
        assert!(!got.flags.contains(ModifierFlags::SHIFT));
        let got = invert_shift(&ev(Key::BackSlash, ModifierFlags::CONTROL));
        assert!(got.flags.contains(ModifierFlags::CONTROL));
        assert!(got.flags.contains(ModifierFlags::SHIFT));
    }
}
