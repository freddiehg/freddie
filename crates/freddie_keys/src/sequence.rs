//! An ordered run of keys, typed with no modifiers, that the caller acts on when it completes.

use crate::{Key, KeyEvent, KeyPress, PressType};

/// A run of keys that means something other than what it types: `jk`, say.
///
/// Each key is swallowed as it arrives, so nothing reaches the app until the run breaks, when the
/// swallowed keys replay in order, or completes, when they are dropped and the caller acts.
///
/// The run demands its keys bare, and takes them rolled: any modifier flag breaks it, but the next
/// key may go down before the one before it comes up.
#[derive(Debug, PartialEq, Eq)]
pub struct KeySequence {
    keys: &'static [Key],
    /// What the run has swallowed, in arrival order; empty when it is idle. Every `Down` in it
    /// matched the next key of `keys`, so counting them is how far the run has got, and every `Up`
    /// belongs to a key already matched. Rolling puts several keys down at once, so the two
    /// interleave and only the order they arrived in can replay them.
    swallowed: Vec<KeyPress>,
}

/// What one key did to a [`KeySequence`].
#[derive(Debug, PartialEq, Eq)]
pub enum KeySequenceOutcome {
    /// The key belongs to the run: it was swallowed, and nothing is emitted.
    Advanced,
    /// The key is not part of the run. These presses replay, in order, and then the key itself,
    /// which the caller emits, since it alone knows the flags that key carried. Empty when the run
    /// was idle, which is every ordinary keystroke.
    Passed(Vec<KeyPress>),
    /// The last key landed. Everything swallowed is dropped and the caller acts on the run.
    Completed,
}

impl KeySequence {
    /// A sequence of `keys`, idle.
    ///
    /// # Panics
    ///
    /// If `keys` is empty.
    #[must_use]
    pub const fn new(keys: &'static [Key]) -> Self {
        assert!(!keys.is_empty(), "a sequence needs at least one key");
        Self {
            keys,
            swallowed: Vec::new(),
        }
    }

    /// Whether the run is idle: it has swallowed nothing.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.swallowed.is_empty()
    }

    /// Feed one key to the run.
    pub fn advance(&mut self, ev: &KeyEvent) -> KeySequenceOutcome {
        if !ev.flags.is_empty() {
            return KeySequenceOutcome::Passed(self.interrupt());
        }
        // Never `keys.len()`: the key that matches the last slot completes the run and clears it.
        let matched = self.matched();
        match ev.press {
            // The next key of the run. The one before it may still be down, which is what a roll
            // is. The key itself must NOT still be down: a sequence may repeat a key (`[j, j]`),
            // and the only thing separating a deliberate second press from a held key's
            // auto-repeat is that the deliberate one came up first.
            PressType::Down if ev.key == self.keys[matched] && !self.is_down(ev.key) => {
                self.swallowed.push(ev.key.down());
                if matched + 1 == self.keys.len() {
                    self.swallowed.clear();
                    KeySequenceOutcome::Completed
                } else {
                    KeySequenceOutcome::Advanced
                }
            }
            // A key the run took, coming up.
            PressType::Up if self.is_down(ev.key) => {
                self.swallowed.push(ev.key.up());
                KeySequenceOutcome::Advanced
            }
            _ => KeySequenceOutcome::Passed(self.interrupt()),
        }
    }

    /// End the run and hand back what it swallowed, in arrival order, leaving it idle. `advance`
    /// calls it for a key that breaks the run; a caller calls it when something outside the keys
    /// ends it, either a key the caller bound itself or a window elapsing.
    pub fn interrupt(&mut self) -> Vec<KeyPress> {
        std::mem::take(&mut self.swallowed)
    }

    /// How many keys of the run have matched: one per `Down`, since every `Up` in `swallowed`
    /// belongs to a key already matched.
    fn matched(&self) -> usize {
        self.swallowed
            .iter()
            .filter(|p| p.press == PressType::Down)
            .count()
    }

    /// Whether the run took `key` and has not seen it come up.
    fn is_down(&self, key: Key) -> bool {
        self.swallowed
            .iter()
            .rev()
            .find(|p| p.key == key)
            .is_some_and(|p| p.press == PressType::Down)
    }
}
