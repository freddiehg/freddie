use std::fmt;

use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

/// A physical hold key that taps as `alone` (+ flags) when released with no other key, and acts
/// as `modifier`/`flag` when another key arrives while it is down.
///
/// No timer. Alone vs modifier is only whether another key arrived before release.
///
/// Examples:
/// - hold `CapsLock`, alone Escape, modifier Control
/// - hold `ShiftLeft`, alone `(`, modifier Shift
/// - hold `ShiftRight`, alone `)`, modifier Shift
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AloneOrModifier {
    hold: Key,
    alone: Key,
    alone_flags: ModifierFlags,
    modifier: Key,
    flag: ModifierFlags,
    role: Role,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Role {
    #[default]
    Idle,
    /// Hold key is down; no other key yet. Up → tap `alone`.
    Pending,
    /// Another key arrived while held; `modifier` down has been emitted.
    AsModifier,
}

impl fmt::Debug for AloneOrModifier {
    /// Only the live phase: `AloneOrModifier { CapsLock: Pending }`. Idle is empty braces.
    ///
    /// Hold/alone/modifier config never changes for a given machine, so printing it on every
    /// dispatch would repeat the definition rather than the state that moved.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AloneOrModifier {{")?;
        match self.role {
            Role::Idle => f.write_str("}"),
            role => write!(f, " {:?}: {:?} }}", self.hold, role),
        }
    }
}

impl AloneOrModifier {
    #[must_use]
    pub const fn new(
        hold: Key,
        alone: Key,
        alone_flags: ModifierFlags,
        modifier: Key,
        flag: ModifierFlags,
    ) -> Self {
        Self {
            hold,
            alone,
            alone_flags,
            modifier,
            flag,
            role: Role::Idle,
        }
    }

    /// `CapsLock` alone → Escape; held with other keys → Control.
    #[must_use]
    pub const fn caps_esc_control() -> Self {
        Self::new(
            Key::CapsLock,
            Key::Escape,
            ModifierFlags::empty(),
            Key::ControlLeft,
            ModifierFlags::CONTROL,
        )
    }

    /// Left shift alone → `(`; held with other keys → real left shift.
    #[must_use]
    pub const fn left_shift_open_paren() -> Self {
        Self::new(
            Key::ShiftLeft,
            Key::Num9,
            ModifierFlags::SHIFT,
            Key::ShiftLeft,
            ModifierFlags::SHIFT,
        )
    }

    /// Right shift alone → `)`; held with other keys → real right shift.
    #[must_use]
    pub const fn right_shift_close_paren() -> Self {
        Self::new(
            Key::ShiftRight,
            Key::Num0,
            ModifierFlags::SHIFT,
            Key::ShiftRight,
            ModifierFlags::SHIFT,
        )
    }

    /// The physical key this dual-role owns.
    #[must_use]
    pub const fn hold(self) -> Key {
        self.hold
    }

    /// Whether the hold is acting as the modifier (another key already promoted this hold).
    #[must_use]
    pub const fn is_modifier(self) -> bool {
        matches!(self.role, Role::AsModifier)
    }

    /// Whether the hold is down and no other key has promoted yet.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self.role, Role::Pending)
    }

    /// Whether the hold is live (pending or acting as modifier).
    #[must_use]
    pub const fn is_held(self) -> bool {
        !matches!(self.role, Role::Idle)
    }

    /// Down or up of the physical hold key. Returns synthetic events to emit; the physical hold
    /// key is never re-emitted (consumer swallows it).
    ///
    /// - Down: enter Pending, emit nothing.
    /// - Up while Pending: `alone` down+up (with `alone_flags`), return Idle.
    /// - Up while acting as modifier: `modifier` up, return Idle.
    /// - Up while Idle: nothing (spurious).
    pub fn on_hold(&mut self, press: PressType) -> Vec<KeyEvent> {
        match press {
            PressType::Down => {
                self.role = Role::Pending;
                Vec::new()
            }
            PressType::Up => match std::mem::replace(&mut self.role, Role::Idle) {
                Role::Pending => vec![
                    KeyEvent {
                        key: self.alone,
                        press: PressType::Down,
                        flags: self.alone_flags,
                    },
                    KeyEvent {
                        key: self.alone,
                        press: PressType::Up,
                        flags: self.alone_flags,
                    },
                ],
                Role::AsModifier => vec![KeyEvent {
                    key: self.modifier,
                    press: PressType::Up,
                    flags: ModifierFlags::empty(),
                }],
                Role::Idle => Vec::new(),
            },
        }
    }

    /// Call before emitting any other key while this dual-role is in force.
    ///
    /// If pending, promotes to modifier and returns `modifier` down (emit first). Otherwise empty.
    pub fn promote_if_pending(&mut self) -> Vec<KeyEvent> {
        if self.role != Role::Pending {
            return Vec::new();
        }
        self.role = Role::AsModifier;
        vec![KeyEvent {
            key: self.modifier,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        }]
    }

    /// Or this dual-role's modifier flag onto `flags` when acting as the modifier.
    #[must_use]
    pub fn stamp(self, flags: ModifierFlags) -> ModifierFlags {
        if matches!(self.role, Role::AsModifier) {
            flags | self.flag
        } else {
            flags
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alone_is_escape() {
        let mut c = AloneOrModifier::caps_esc_control();
        assert!(c.on_hold(PressType::Down).is_empty());
        assert!(c.is_pending());
        let out = c.on_hold(PressType::Up);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, Key::Escape);
        assert_eq!(out[0].press, PressType::Down);
        assert_eq!(out[1].key, Key::Escape);
        assert_eq!(out[1].press, PressType::Up);
        assert!(!c.is_modifier());
        assert!(!c.is_pending());
    }

    #[test]
    fn with_other_key_is_control() {
        let mut c = AloneOrModifier::caps_esc_control();
        assert!(c.on_hold(PressType::Down).is_empty());
        let prefix = c.promote_if_pending();
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix[0].key, Key::ControlLeft);
        assert_eq!(prefix[0].press, PressType::Down);
        assert!(c.is_modifier());
        let flags = c.stamp(ModifierFlags::empty());
        assert!(flags.contains(ModifierFlags::CONTROL));
        assert!(c.promote_if_pending().is_empty());
        let release = c.on_hold(PressType::Up);
        assert_eq!(release.len(), 1);
        assert_eq!(release[0].key, Key::ControlLeft);
        assert_eq!(release[0].press, PressType::Up);
    }

    #[test]
    fn left_shift_alone_is_open_paren() {
        let mut s = AloneOrModifier::left_shift_open_paren();
        assert_eq!(s.hold(), Key::ShiftLeft);
        assert!(s.on_hold(PressType::Down).is_empty());
        let out = s.on_hold(PressType::Up);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, Key::Num9);
        assert!(out[0].flags.contains(ModifierFlags::SHIFT));
        assert_eq!(out[1].key, Key::Num9);
        assert!(out[1].flags.contains(ModifierFlags::SHIFT));
    }

    #[test]
    fn left_shift_with_letter_is_real_shift() {
        let mut s = AloneOrModifier::left_shift_open_paren();
        assert!(s.on_hold(PressType::Down).is_empty());
        let prefix = s.promote_if_pending();
        assert_eq!(prefix[0].key, Key::ShiftLeft);
        assert!(
            s.stamp(ModifierFlags::empty())
                .contains(ModifierFlags::SHIFT)
        );
    }

    #[test]
    fn idle_up_is_noop() {
        let mut c = AloneOrModifier::caps_esc_control();
        assert!(c.on_hold(PressType::Up).is_empty());
    }

    #[test]
    fn debug_prints_phase_only() {
        let mut c = AloneOrModifier::caps_esc_control();
        assert_eq!(format!("{c:?}"), "AloneOrModifier {}");
        let _ = c.on_hold(PressType::Down);
        assert_eq!(format!("{c:?}"), "AloneOrModifier { CapsLock: Pending }");
        let _ = c.promote_if_pending();
        assert_eq!(format!("{c:?}"), "AloneOrModifier { CapsLock: AsModifier }");
    }
}
