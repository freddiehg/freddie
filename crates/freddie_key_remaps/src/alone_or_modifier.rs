use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

/// A physical hold key that taps as `alone` when released with no other key, and acts as
/// `modifier`/`flag` when another key arrives while it is down.
///
/// No timer. Alone vs modifier is only whether another key arrived before release.
///
/// Classic instance: hold key = `CapsLock`, `alone` = Escape, `modifier` = `ControlLeft` + CONTROL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AloneOrModifier {
    alone: Key,
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

impl AloneOrModifier {
    #[must_use]
    pub const fn new(alone: Key, modifier: Key, flag: ModifierFlags) -> Self {
        Self {
            alone,
            modifier,
            flag,
            role: Role::Idle,
        }
    }

    /// `CapsLock` alone → Escape; held with other keys → Control.
    #[must_use]
    pub const fn caps_esc_control() -> Self {
        Self::new(Key::Escape, Key::ControlLeft, ModifierFlags::CONTROL)
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
    /// - Up while Pending: `alone` down+up, return Idle.
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
                        flags: ModifierFlags::empty(),
                    },
                    KeyEvent {
                        key: self.alone,
                        press: PressType::Up,
                        flags: ModifierFlags::empty(),
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
    fn idle_up_is_noop() {
        let mut c = AloneOrModifier::caps_esc_control();
        assert!(c.on_hold(PressType::Up).is_empty());
    }
}
