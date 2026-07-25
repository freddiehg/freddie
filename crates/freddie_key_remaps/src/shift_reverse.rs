use freddie_keys::{KeyEvent, ModifierFlags};

/// Swap bare ↔ shift-only on a key event: bare becomes SHIFT, SHIFT alone becomes bare.
///
/// Any other flag combination (cmd, ctrl, shift+cmd, …) returns `None` so the caller leaves
/// the event alone. The physical key and press type are unchanged.
///
/// Used for number-row invert (`1` ↔ `!`), backslash ↔ pipe, and similar.
#[must_use]
pub fn shift_reverse(ev: &KeyEvent) -> Option<KeyEvent> {
    let out_flags = if ev.flags == ModifierFlags::empty() {
        ModifierFlags::SHIFT
    } else if ev.flags == ModifierFlags::SHIFT {
        ModifierFlags::empty()
    } else {
        return None;
    };
    Some(KeyEvent {
        key: ev.key,
        press: ev.press,
        flags: out_flags,
    })
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
        let got = shift_reverse(&ev(Key::Num1, ModifierFlags::empty())).expect("bare");
        assert_eq!(got.key, Key::Num1);
        assert!(got.flags.contains(ModifierFlags::SHIFT));
    }

    #[test]
    fn shift_only_drops_shift() {
        let got = shift_reverse(&ev(Key::Num1, ModifierFlags::SHIFT)).expect("shift");
        assert_eq!(got.key, Key::Num1);
        assert!(got.flags.is_empty());
    }

    #[test]
    fn other_flags_skip() {
        assert!(shift_reverse(&ev(Key::Num1, ModifierFlags::COMMAND)).is_none());
        assert!(
            shift_reverse(&ev(
                Key::Num1,
                ModifierFlags::SHIFT | ModifierFlags::COMMAND
            ))
            .is_none()
        );
    }
}
