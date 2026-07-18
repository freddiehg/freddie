//! An ordered run of keys, typed with no modifiers, that the caller acts on when it completes.

use std::time::Duration;

use freddie_keys::{Key, KeyEvent, KeyPress, PressType};

use crate::TimerGuard;

/// A run of keys that means something other than what it types: `jk`, say.
///
/// Each key is swallowed as it arrives, so nothing reaches the app until the run breaks, when the
/// swallowed keys replay in order, or completes, when they are dropped and the caller acts.
///
/// The run demands its keys bare, and takes them rolled: any modifier flag breaks it, but the next
/// key may go down before the one before it comes up.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
pub struct KeySequence {
    keys: &'static [Key],
    /// The window a run of this sequence waits, and the guard for it while one is live. `None`
    /// for a sequence that waits forever.
    window: Option<Window>,
    /// What the run has swallowed, in arrival order; empty when it is idle. Every `Down` in it
    /// matched the next key of `keys`, so counting them is how far the run has got, and every `Up`
    /// belongs to a key already matched. Rolling puts several keys down at once, so the two
    /// interleave and only the order they arrived in can replay them.
    swallowed: Vec<KeyPress>,
}

/// How long a run waits for its next key, and what cancels that wait.
///
/// The two are one field because a guard without a duration is nonsense: there would be nothing
/// to have armed. Two `Option`s side by side would let that state exist.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
struct Window {
    duration: Duration,
    /// Armed while a run is live, dropped when it ends, which is what cancels the wait.
    timer: Option<TimerGuard>,
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

impl std::fmt::Debug for KeySequence {
    /// Only what the run has swallowed, in arrival order: `KeySequence { KeyJ v, KeyJ ^ }`, or
    /// `KeySequence {}` when idle.
    ///
    /// It is written on every dispatched event, and the keys and the window never change, so
    /// printing them would repeat the sequence's definition on every line of the log to say
    /// nothing about what happened.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeySequence {{")?;
        for (i, press) in self.swallowed.iter().enumerate() {
            let arrow = match press.press {
                PressType::Down => "v",
                PressType::Up => "^",
            };
            write!(
                f,
                "{}{:?} {arrow}",
                if i == 0 { " " } else { ", " },
                press.key
            )?;
        }
        f.write_str(if self.swallowed.is_empty() { "}" } else { " }" })
    }
}

impl KeySequence {
    /// A sequence of `keys`, idle.
    ///
    /// # Panics
    ///
    /// If `keys` is empty.
    #[must_use]
    pub fn new(keys: &'static [Key], window: Option<Duration>) -> Self {
        assert!(!keys.is_empty(), "a sequence needs at least one key");
        Self {
            keys,
            window: window.map(|duration| Window {
                duration,
                timer: None,
            }),
            swallowed: Vec::new(),
        }
    }

    /// How long a run of this sequence waits for its next key, or `None` if it waits forever.
    #[must_use]
    pub fn window(&self) -> Option<Duration> {
        self.window.as_ref().map(|w| w.duration)
    }

    /// Give the run in progress the guard for its window, so the wait is cancelled by the run
    /// ending rather than by the caller remembering to.
    ///
    /// # Panics
    ///
    /// If no run is in progress, since nothing would ever drop the guard, or if this sequence has
    /// no window, since then there was nothing to arm.
    pub fn hold(&mut self, guard: TimerGuard) {
        assert!(!self.is_idle(), "an idle run has no life to tie a guard to");
        let window = self
            .window
            .as_mut()
            .expect("a sequence with no window cannot have armed one");
        window.timer = Some(guard);
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
                    self.disarm();
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
        self.disarm();
        std::mem::take(&mut self.swallowed)
    }

    /// Drop the guard, cancelling the wait, and leave the duration in place: the sequence still
    /// has a window, it is just not running one.
    fn disarm(&mut self) {
        if let Some(window) = self.window.as_mut() {
            window.timer = None;
        }
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
