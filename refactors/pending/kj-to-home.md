# jk leaves typing for home

In the typing layer, `j` then `k` returns to home; neither key reaches the app. A `j` not followed by `k` types itself.

The machine is a general one, `KeySequence`, in `freddie_keys`: an ordered run of keys, swallowed as they arrive, replayed exactly if the run breaks and dropped if it completes. mercury holds one of them, `[j, k]`, in its `TypingState`, and turns a completed run into a layer change. v0 builds the sequence and the binding; v1 adds a timeout.

Two rules bound the version we are building:

- No modifiers. Any flag on an incoming key breaks the run, so `cmd`-`j` types `cmd`-`j` and a `k` under `shift` is not the second half of a `jk`. Every key a sequence swallows therefore arrived bare, so a replay is always `ModifierFlags::empty()` and the state stores no flags.
- Rolled or deliberate, both count. `j` down, `j` up, `k` down is the run; so is `j` down, `k` down, `j` up, `k` up, which is what typing it at speed produces. The next key may go down while the one before it is still down, so several keys can be down at once and their ups interleave. The state is therefore what was swallowed, in arrival order, so that a break replays the stream as it happened rather than a reconstruction of it.

Exactly two events keep a run alive, in whatever order they arrive:

- the down of its next key, when that key is not already down;
- the up of a key it already swallowed and has not seen come up.

Everything else breaks it, a held key's auto-repeat included: a repeat is a down of a key that is already down, so it never counts, and holding `j` types `jjjj` the way it would if nothing were watching. That "not already down" is also what lets a sequence repeat a key. `[j, j]` fires on a double tap and not on a held `j`, because the deliberate second press is the one that came up first.

A key that breaks a run is typed and nothing more: it does not open a new one. `j`, `j`, `k` types `jjk` rather than typing `j` and then leaving for home, because the second `j` is what broke the first run. The caller emits the breaking key itself, so the run never has to say "I took this one after all".

# v0: the sequence

## change 1: the KeySequence primitive

`crates/freddie_keys/src/sequence.rs`, new, and `mod sequence; pub use sequence::{KeySequence, KeySequenceOutcome};` in `crates/freddie_keys/src/lib.rs`.

```rust
//! An ordered run of keys, typed with no modifiers, that the caller acts on when it completes.

use crate::{Key, KeyEvent, KeyPress, PressType};

/// A run of keys that means something other than what it types: `jk`, say. Each key is swallowed
/// as it arrives, so nothing reaches the app until the run breaks, when the swallowed keys replay
/// in order, or completes, when they are dropped and the caller acts.
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
    pub fn is_idle(&self) -> bool {
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
```

`ModifierFlags` gains the test the gate reads, in `crates/freddie_keys/src/lib.rs` beside `empty`:

```rust
    /// Whether no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
```

## change 2: TypingState holds the jk sequence

`crates/mercury/src/state/mod.rs`. `TypingState` stops deriving `Default`, because a `KeySequence` has no meaningful empty value: it is defined by its keys.

before:

```rust
#[derive(Debug, Default)]
pub struct TypingState {
    /// The physical truth about which modifier keys are down [..]
    pub held: HeldModifiers,
}
```

after:

```rust
/// The keys that leave typing for home.
const JK: &[Key] = &[Key::KeyJ, Key::KeyK];

#[derive(Debug)]
pub struct TypingState {
    /// The physical truth about which modifier keys are down [..]
    pub held: HeldModifiers,
    /// The `jk` run. Replaced with a fresh one on every layer change, so a hold never outlives the
    /// layer it was typed in.
    pub jk: KeySequence,
}

impl Default for TypingState {
    fn default() -> Self {
        Self {
            held: HeldModifiers::default(),
            jk: KeySequence::new(JK),
        }
    }
}
```

`set_layer`, before:

```rust
        self.layer = into;
        match (before_passthrough, after_passthrough) {
```

after:

```rust
        self.layer = into;
        self.typing_state.jk = KeySequence::new(JK);
        match (before_passthrough, after_passthrough) {
```

`state/mod.rs` imports `KeySequence` from `freddie_keys`.

## change 3: the handler runs the sequence

`crates/mercury/src/handlers/root.rs`, `maybe_pass_through`, before:

```rust
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, ev.flags)]
    } else {
        Vec::new()
    }
}
```

after:

```rust
pub(crate) fn maybe_pass_through(
    ev: &KeyEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent;
    if ev.key.is_modifier() {
        root.typing_state.held.apply(ev);
    }
    if !root.layer().is_passthrough() {
        return Vec::new();
    }
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced => Vec::new(),
        KeySequenceOutcome::Passed(presses) => {
            let mut out = replay(presses);
            out.push(emit(ev.key, ev.press, ev.flags));
            out
        }
        KeySequenceOutcome::Completed => root.set_layer(HomeLayer::new()),
    }
}
```

`root.rs` imports `HomeLayer`, `KeySequenceOutcome`, and `replay`.

`crates/mercury/src/effect.rs`, beside `emit`, since two handlers replay a broken run:

```rust
/// The effects that replay what a sequence swallowed. Every key it took arrived with no modifier,
/// so every one of them goes back out bare.
pub(crate) fn replay(presses: Vec<KeyPress>) -> Vec<MercuryEffect> {
    presses
        .into_iter()
        .map(|p| emit(p.key, p.press, ModifierFlags::empty()))
        .collect()
}
```

## change 4: the keys typing binds break the run

`TypingLayer` binds `escape` (`crates/mercury/src/state/typing.rs`), and a key the active layer binds never reaches the root, so it never reaches the sequence. Without this, a plain escape typed while a `j` is held is emitted ahead of that `j`, and the `j` replays after it.

`crates/mercury/src/handlers/typing.rs`, `maybe_go_home`, before:

```rust
    let root: MercuryPath<'_> = node.parent.ascend();
    if root.typing_state.held.meta.any_held() {
        root.set_layer(HomeLayer::new())
    } else {
        vec![emit(ev.key, ev.press, root.typing_state.held.flags())]
    }
```

after:

```rust
    let root: MercuryPath<'_> = node.parent.ascend();
    if root.typing_state.held.meta.any_held() {
        // `set_layer` replaces the run, so a held `j` is abandoned rather than typed. Replaying it
        // would strand the key down: the app would see a down whose up lands in a command layer
        // and is swallowed there.
        root.set_layer(HomeLayer::new())
    } else {
        let mut out = replay(root.typing_state.jk.interrupt());
        out.push(emit(ev.key, ev.press, root.typing_state.held.flags()));
        out
    }
```

`typing.rs` imports `replay`.

## change 5: tests

`crates/freddie_keys/tests/sequence.rs`, new, over the primitive itself: a completed run, a broken one at each point it can break, a modifier breaking it, a roll breaking it, an auto-repeat swallowed, and a key that is not in the run at all passing with an empty replay.

`crates/mercury/tests/transitions.rs`, extending the typing table. `key(..)` builds a down; add `up(..)` for the release halves, and `key_with(.., flags)` for a key carrying a modifier.

```rust
// jk, typed one key at a time, leaves for home and types nothing.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert!(!m.typing_state.jk.is_idle());
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&key(Key::KeyK)), Some(vec![]));
assert!(matches!(m.layer(), Layer::Home(_)));
assert!(m.typing_state.jk.is_idle());

// Rolled, k down before j comes up: the run completes the same way, and the two ups that follow
// arrive in Home, which binds neither and is not a passthrough layer, so they are swallowed
// rather than reaching the app as ups with no downs.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&key(Key::KeyK)), Some(vec![]));
assert!(matches!(m.layer(), Layer::Home(_)));
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&up(Key::KeyK)), Some(vec![]));

// The auto-repeat of a held j breaks the run: the swallowed down replays ahead of the repeat, so
// the app sees the same two downs it would have seen unwatched, and the run is idle for the rest.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key(Key::KeyJ)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
    ]),
);
assert!(m.typing_state.jk.is_idle());
assert_eq!(
    m.handle(&key(Key::KeyK)),
    Some(vec![emit(Key::KeyK, PressType::Down, ModifierFlags::empty())]),
);
assert!(matches!(m.layer(), Layer::Typing(_)));

// j, j, k types all three: the second j breaks the first run and does not open a second.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key(Key::KeyJ)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::KeyJ, PressType::Up, ModifierFlags::empty()),
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
    ]),
);
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![emit(
    Key::KeyJ,
    PressType::Up,
    ModifierFlags::empty()
)]));
assert_eq!(
    m.handle(&key(Key::KeyK)),
    Some(vec![emit(Key::KeyK, PressType::Down, ModifierFlags::empty())]),
);
assert!(matches!(m.layer(), Layer::Typing(_)));

// j then a: the whole j tap replays ahead of the a.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key(Key::KeyA)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::KeyJ, PressType::Up, ModifierFlags::empty()),
        emit(Key::KeyA, PressType::Down, ModifierFlags::empty()),
    ]),
);

// j held, then a: only the j down has been swallowed, so only it replays. The real j up passes
// through later, with the run already idle.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key(Key::KeyA)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::KeyA, PressType::Down, ModifierFlags::empty()),
    ]),
);
assert_eq!(
    m.handle(&up(Key::KeyJ)),
    Some(vec![emit(Key::KeyJ, PressType::Up, ModifierFlags::empty())]),
);

// A j carrying a modifier never opens the run.
let mut m = Mercury::default();
assert_eq!(
    m.handle(&key_with(Key::KeyJ, ModifierFlags::COMMAND)),
    Some(vec![emit(Key::KeyJ, PressType::Down, ModifierFlags::COMMAND)]),
);
assert!(m.typing_state.jk.is_idle());

// A modifier arriving mid-run breaks it: the held j replays ahead of the modifier.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key_with(Key::MetaLeft, ModifierFlags::COMMAND)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::MetaLeft, PressType::Down, ModifierFlags::COMMAND),
    ]),
);

// A plain escape is bound by the typing layer, so it never reaches the root. It still breaks the
// run, and the j replays AHEAD of it rather than after.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert_eq!(
    m.handle(&key(Key::Escape)),
    Some(vec![
        emit(Key::KeyJ, PressType::Down, ModifierFlags::empty()),
        emit(Key::KeyJ, PressType::Up, ModifierFlags::empty()),
        emit(Key::Escape, PressType::Down, ModifierFlags::empty()),
    ]),
);
assert!(matches!(m.layer(), Layer::Typing(_)));

// cmd-escape leaves for home, and the held j is abandoned: nothing of it is typed, and the only
// effect is the sweep releasing the cmd that was passed through.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
let _ = m.handle(&key_with(Key::MetaLeft, ModifierFlags::COMMAND));
assert_eq!(
    m.handle(&key(Key::Escape)),
    Some(vec![emit(Key::MetaLeft, PressType::Up, ModifierFlags::empty())]),
);
assert!(matches!(m.layer(), Layer::Home(_)));
assert!(m.typing_state.jk.is_idle());
```

# v1: the timeout

A run waits `JK_TIMEOUT` for its next key; on expiry what was swallowed replays and the run resets, so a later `k` is an ordinary `k`. The guard lives in mercury next to the sequence rather than inside it, so `freddie_keys` stays free of `freddie`'s timer types.

Builds on `refactors/past/timer-events.md` and `refactors/past/layer-timeout.md`.

## change 1: the timeout trigger

`crates/mercury/src/sources.rs`, appended:

```rust
/// The `jk` run's timeout. It carries nothing, so one type is both the trigger and the event.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct JkTimeout;

bind::self_trigger!(JkTimeout);
```

`model.rs` adds `JkTimeout(JkTimeout)` to `MercuryTrigger` and `MercuryEvent`; its `use crate::{..}` picks up `JkTimeout`. `lib.rs` adds `JkTimeout` to the `sources` re-export.

## change 2: the duration and the arm helper

`crates/mercury/src/state/mod.rs`, by `RETURN_TO_HOME_TIMEOUT` and `arm_return_home`:

```rust
/// How long a run waits for its next key before what it swallowed types itself.
pub const JK_TIMEOUT: Duration = Duration::from_millis(500);

/// Arm the `jk` timeout: the guard cancels it on drop, the effect schedules it.
fn arm_jk_timeout() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(JK_TIMEOUT, MercuryEvent::JkTimeout(JkTimeout));
    (guard, MercuryEffect::Timer(effect))
}
```

The `use crate::{..}` in `state/mod.rs` adds `JkTimeout`; `JK_TIMEOUT` joins the `pub use` through `state/mod.rs` and `lib.rs`.

## change 3: TypingState holds the guard

`crates/mercury/src/state/mod.rs`, `TypingState`, before:

```rust
pub struct TypingState {
    pub held: HeldModifiers,
    pub jk: KeySequence,
}
```

after:

```rust
pub struct TypingState {
    pub held: HeldModifiers,
    pub jk: KeySequence,
    /// Live exactly while `jk` is mid-run: dropping it cancels the timeout. Written only by
    /// `maybe_pass_through`, in the same match that reads the run's outcome, so the two cannot
    /// disagree.
    jk_timer: Option<TimerGuard>,
}
```

`Default for TypingState` adds `jk_timer: None`, and `set_layer`'s `self.typing_state.jk = KeySequence::new(JK)` is followed by `self.typing_state.jk_timer = None`.

## change 4: the handler arms and disarms

`crates/mercury/src/handlers/root.rs`, `maybe_pass_through`. The run is idle before the key iff this key opens it, so that is when the timer is armed; every other outcome ends the run and drops the guard.

before:

```rust
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced => Vec::new(),
```

after:

```rust
    let opening = root.typing_state.jk.is_idle();
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced if opening => {
            let (guard, timer) = arm_jk_timeout();
            root.typing_state.jk_timer = Some(guard);
            vec![timer]
        }
        KeySequenceOutcome::Advanced => Vec::new(),
```

and the other two arms clear it:

```rust
        KeySequenceOutcome::Passed(replay) => {
            root.typing_state.jk_timer = None;
            replay
                .into_iter()
                .map(|p| emit(p.key, p.press, ModifierFlags::empty()))
                .chain(std::iter::once(emit(ev.key, ev.press, ev.flags)))
                .collect()
        }
        KeySequenceOutcome::Completed => {
            root.typing_state.jk_timer = None;
            root.set_layer(HomeLayer::new())
        }
```

## change 5: the root binds the timeout

`crates/mercury/src/state/mod.rs`, `Mercury`'s `#[bind(..)]`, before:

```rust
    Quit => quit,
    AnyKey => maybe_pass_through,
```

after:

```rust
    Quit => quit,
    JkTimeout => jk_timeout,
    AnyKey => maybe_pass_through,
```

`crates/mercury/src/handlers/root.rs`, new handler:

```rust
/// The window elapsed with no next key: what the run swallowed types itself.
pub(crate) fn jk_timeout(_ev: &JkTimeout, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.typing_state.jk_timer = None;
    root.typing_state
        .jk
        .interrupt()
        .into_iter()
        .map(|p| emit(p.key, p.press, ModifierFlags::empty()))
        .collect()
}
```

`root.rs` adds `JkTimeout` to its imports. The guard is dropped whenever the run ends, so a `JkTimeout` for a run that already ended never fires; one that arrives anyway finds an idle run and emits nothing.

## change 6: tests

`crates/mercury/tests/transitions.rs`. The v0 cases hold, except that the key which opens the run now also returns the `Timer` effect; rebuild the expected `Timer` to assert it (its `testing` equality is the delay and the fire event).

```rust
let (_guard, effect) =
    freddie::timer_effect_and_guard(JK_TIMEOUT, MercuryEvent::JkTimeout(JkTimeout));
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![MercuryEffect::Timer(effect)]));

// The window elapses with j still down: its down types itself.
assert_eq!(
    m.handle(&MercuryEvent::JkTimeout(JkTimeout)),
    Some(vec![emit(Key::KeyJ, PressType::Down, ModifierFlags::empty())]),
);
assert!(m.typing_state.jk.is_idle());

// A JkTimeout with no run in progress emits nothing.
let mut m = Mercury::default();
assert_eq!(m.handle(&MercuryEvent::JkTimeout(JkTimeout)), Some(vec![]));
```
