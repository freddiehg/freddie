# jk leaves typing for home

In the typing layer, `j` then `k` returns to home; neither key reaches the app. A `j` not followed by `k` types itself. The passthrough state (the held modifiers and the pending `j`) lives in the root's `TypingState`; `TypingState::process` runs the sequence. v0 adds the `jk` state machine, v1 a timeout.

Holding a `j` swallows its down and its up. The key and the direction are fixed by the state, so the only thing a replay needs is the flags the `j` arrived with, which the state carries. `JDown` replays the down alone, while `j` is still physically down; `JUp` replays down and up, a full tap. The flags are captured at the `j` down rather than read from `held` at flush time: `held` has already recorded the modifier that caused the flush, and it never carries `FN`, which is not a key and rides in only as a flag on other events. A held modifier suspends the sequence: with any modifier down, `j` passes straight through.

# v0: the jk sequence

## change 1: JkState and the jk_state field

`crates/mercury/src/state/mod.rs`, new. The flags ride from the `j` down through to the flush, and one set covers both halves of the replayed tap: every flag change that mercury can see arrives as a key event, which resolves the hold before it can matter. `fn` is the exception, because its `FlagsChanged` never becomes a `KeyEvent` (`press_of` in `freddie_keyboard`'s macOS backend takes only control/command/alt/shift) and so rides in only as a bit on the next event. A `j` up that gained `FN` mid-hold replays with the down's flags, which is the tap the app should see.

```rust
/// Progress through the `jk` escape sequence, carrying the flags the held `j` arrived with so a
/// flush replays it as it was pressed.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum JkState {
    #[default]
    None,
    /// `j` down was swallowed; `j` is still physically down.
    JDown(ModifierFlags),
    /// `j` down and up were both swallowed; `j` is physically up.
    JUp(ModifierFlags),
}
```

`TypingState` gains the field, before:

```rust
pub struct TypingState {
    pub held: HeldModifiers,
}
```

after:

```rust
pub struct TypingState {
    pub held: HeldModifiers,
    /// A `j` held in typing, pending a `k`. Cleared on every layer change.
    pub jk_state: JkState,
}
```

`set_layer` clears it, before:

```rust
        self.layer = into;
        match (before_passthrough, after_passthrough) {
```

after:

```rust
        self.layer = into;
        self.typing_state.jk_state = JkState::None;
        match (before_passthrough, after_passthrough) {
```

`state/mod.rs` and `lib.rs` add `JkState` to their `pub use`.

## change 2: the sequence logic

`crates/mercury/src/state/mod.rs`. `TypingState::process` is the entry: a held modifier suspends the sequence, otherwise `JkState::advance` runs it. `advance` returns the effects, or `GoHome` for the caller to leave to home; `flush` drains the held `j` and resets. `emit`, `Key`, `KeyEvent`, and `PressType` are already in scope.

```rust
/// The result of feeding one key to [`JkState::advance`].
pub(crate) enum JkOutcome {
    /// Emit these and stay in typing.
    Emit(Vec<MercuryEffect>),
    /// `jk` completed: leave to home.
    GoHome,
}

impl TypingState {
    /// Run the passthrough sequence for one key. A held modifier suspends `jk`, so flush any held
    /// `j` and pass the key through; otherwise advance the `jk` state.
    pub(crate) fn process(&mut self, ev: &KeyEvent) -> JkOutcome {
        if self.held.any_held() {
            let mut out = self.jk_state.flush();
            out.push(emit(ev.key, ev.press, ev.flags));
            return JkOutcome::Emit(out);
        }
        self.jk_state.advance(ev)
    }
}

impl JkState {
    /// Replay the held `j` and reset: its down while `j` is still physically down, the whole tap
    /// once its up has been swallowed.
    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        match std::mem::take(self) {
            Self::None => Vec::new(),
            Self::JDown(flags) => vec![emit(Key::KeyJ, PressType::Down, flags)],
            Self::JUp(flags) => vec![
                emit(Key::KeyJ, PressType::Down, flags),
                emit(Key::KeyJ, PressType::Up, flags),
            ],
        }
    }

    /// Advance the sequence for one key (no modifier held).
    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (&*self, ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                *self = Self::JDown(ev.flags);
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(flags), Key::KeyJ, PressType::Up) => {
                *self = Self::JUp(*flags);
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(_), Key::KeyJ, PressType::Down) => JkOutcome::Emit(Vec::new()),
            (Self::JDown(_) | Self::JUp(_), Key::KeyK, PressType::Down) => JkOutcome::GoHome,
            (Self::JDown(_) | Self::JUp(_), ..) => {
                let mut out = self.flush();
                out.push(emit(ev.key, ev.press, ev.flags));
                JkOutcome::Emit(out)
            }
            (Self::None, ..) => JkOutcome::Emit(vec![emit(ev.key, ev.press, ev.flags)]),
        }
    }
}
```

`HeldModifiers` gains `any_held`:

```rust
    /// Whether any tracked modifier (control/command/alt/shift) is down.
    #[must_use]
    pub const fn any_held(&self) -> bool {
        self.control.any_held()
            || self.meta.any_held()
            || self.alt.any_held()
            || self.shift.any_held()
    }
```

`JkOutcome` joins the `state/mod.rs` `pub use`.

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
    match root.typing_state.process(ev) {
        JkOutcome::Emit(effects) => effects,
        JkOutcome::GoHome => root.set_layer(HomeLayer::new()),
    }
}
```

`root.rs` imports `HomeLayer` and `JkOutcome`.

## change 4: tests

`crates/mercury/tests/transitions.rs`, extending the typing table. `key(..)` builds a down; add `up(..)` for the release halves. State is checked with `matches!`; a replayed `j` is asserted with the `emit` helper.

```rust
// jk deliberate.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert!(matches!(m.typing_state.jk_state, JkState::JDown(_)));
assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
assert!(matches!(m.typing_state.jk_state, JkState::JUp(_)));
assert_eq!(m.handle(&key(Key::KeyK)), Some(vec![]));
assert!(matches!(m.layer(), Layer::Home(_)));

// jk rolled.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
assert_eq!(m.handle(&key(Key::KeyK)), Some(vec![]));
assert!(matches!(m.layer(), Layer::Home(_)));

// j then a, deliberate: the j tap flushes ahead of a.
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

// j then a, rolled: only j down flushes, its real up passes through later.
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

// j with a modifier held passes straight through.
let mut m = Mercury::default();
m.handle(&key(Key::MetaLeft));
assert_eq!(
    m.handle(&key(Key::KeyJ)),
    Some(vec![emit(Key::KeyJ, PressType::Down, ModifierFlags::COMMAND)]),
);
assert!(matches!(m.typing_state.jk_state, JkState::None));

// j then cmd-escape: the held j is dropped, none emitted.
let mut m = Mercury::default();
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![]));
m.handle(&key(Key::MetaLeft));
assert_eq!(m.handle(&key(Key::Escape)), Some(vec![]));
assert!(matches!(m.layer(), Layer::Home(_)));
assert!(matches!(m.typing_state.jk_state, JkState::None));
```

# v1: the timeout

A held `j` waits `JK_TIMEOUT` for a `k`; on expiry it flushes and resets, so a later `k` is an ordinary `k`. `JDown`/`JUp` carry the `TimerGuard`; dropping it cancels the timer. `TimerGuard` derives `PartialEq`/`Eq` only under `testing`, so `JkState` gates them the same way.

Builds on `refactors/past/timer-events.md` and `refactors/past/layer-timeout.md`.

## change 1: the timeout trigger

`crates/mercury/src/sources.rs`, appended:

```rust
/// The `jk` hold's timeout. It carries nothing, so one type is both the trigger and the event.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct JkTimeout;

bind::self_trigger!(JkTimeout);
```

`model.rs` adds `JkTimeout(JkTimeout)` to `MercuryTrigger` and `MercuryEvent`; its `use crate::{..}` picks up `JkTimeout`. `lib.rs` adds `JkTimeout` to the `sources` re-export.

## change 2: the duration and the arm helper

`crates/mercury/src/state/mod.rs`, by `RETURN_TO_HOME_TIMEOUT` and `arm_return_home`:

```rust
/// How long a held `j` waits for a `k` before it flushes as an ordinary keystroke.
pub const JK_TIMEOUT: Duration = Duration::from_millis(200);

/// Arm the `jk` timeout: the guard cancels it on drop, the effect schedules it.
fn arm_jk_timeout() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(JK_TIMEOUT, MercuryEvent::JkTimeout(JkTimeout));
    (guard, MercuryEffect::Timer(effect))
}
```

The `use crate::{..}` in `state/mod.rs` adds `JkTimeout`; `JK_TIMEOUT` joins the `pub use` through `state/mod.rs` and `lib.rs`.

## change 3: JkState carries the guard and arms it

The flags gain a companion, so the payload becomes a struct rather than a second tuple field. `TimerGuard` is not `Copy`, so `advance` matches on the state BY VALUE (`std::mem::take`) and each arm states what it leaves behind; the arms that resolve the hold leave `None`, and the guard they took drops there.

`crates/mercury/src/state/mod.rs`, `JkState`, before:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub enum JkState {
    #[default]
    None,
    /// `j` down was swallowed; `j` is still physically down.
    JDown(ModifierFlags),
    /// `j` down and up were both swallowed; `j` is physically up.
    JUp(ModifierFlags),
}
```

after:

```rust
/// A `j` held pending a `k`: the flags it arrived with, and the guard whose drop cancels its
/// timeout.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct HeldJ {
    flags: ModifierFlags,
    timer: TimerGuard,
}

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, Default)]
pub enum JkState {
    #[default]
    None,
    /// `j` down was swallowed; `j` is still physically down.
    JDown(HeldJ),
    /// `j` down and up were both swallowed; `j` is physically up. The guard carries over from
    /// `JDown`, so the window runs from the first `j`.
    JUp(HeldJ),
}
```

`flush` splits: `into_replay` consumes the state (and so the guard), and `flush` is that over a `take`. `flush` and `advance`, before:

```rust
    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        match std::mem::take(self) {
            Self::None => Vec::new(),
            Self::JDown(flags) => vec![emit(Key::KeyJ, PressType::Down, flags)],
            Self::JUp(flags) => vec![
                emit(Key::KeyJ, PressType::Down, flags),
                emit(Key::KeyJ, PressType::Up, flags),
            ],
        }
    }

    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (&*self, ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                *self = Self::JDown(ev.flags);
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(flags), Key::KeyJ, PressType::Up) => {
                *self = Self::JUp(*flags);
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(_), Key::KeyJ, PressType::Down) => JkOutcome::Emit(Vec::new()),
            (Self::JDown(_) | Self::JUp(_), Key::KeyK, PressType::Down) => JkOutcome::GoHome,
            (Self::JDown(_) | Self::JUp(_), ..) => {
                let mut out = self.flush();
                out.push(emit(ev.key, ev.press, ev.flags));
                JkOutcome::Emit(out)
            }
            (Self::None, ..) => JkOutcome::Emit(vec![emit(ev.key, ev.press, ev.flags)]),
        }
    }
```

after:

```rust
    /// Replay the held `j`: its down while `j` is still physically down, the whole tap once its up
    /// has been swallowed. It consumes the state, so the guard drops and the timeout is cancelled.
    fn into_replay(self) -> Vec<MercuryEffect> {
        match self {
            Self::None => Vec::new(),
            Self::JDown(held) => vec![emit(Key::KeyJ, PressType::Down, held.flags)],
            Self::JUp(held) => vec![
                emit(Key::KeyJ, PressType::Down, held.flags),
                emit(Key::KeyJ, PressType::Up, held.flags),
            ],
        }
    }

    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        std::mem::take(self).into_replay()
    }

    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (std::mem::take(self), ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                let (timer, effect) = arm_jk_timeout();
                *self = Self::JDown(HeldJ {
                    flags: ev.flags,
                    timer,
                });
                JkOutcome::Emit(vec![effect])
            }
            (Self::JDown(held), Key::KeyJ, PressType::Up) => {
                *self = Self::JUp(held);
                JkOutcome::Emit(Vec::new())
            }
            (held @ Self::JDown(_), Key::KeyJ, PressType::Down) => {
                *self = held;
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(_) | Self::JUp(_), Key::KeyK, PressType::Down) => JkOutcome::GoHome,
            (held @ (Self::JDown(_) | Self::JUp(_)), ..) => {
                let mut out = held.into_replay();
                out.push(emit(ev.key, ev.press, ev.flags));
                JkOutcome::Emit(out)
            }
            (Self::None, ..) => JkOutcome::Emit(vec![emit(ev.key, ev.press, ev.flags)]),
        }
    }
```

The `take` leaves `None` behind, so an arm that resolves the hold is done: `GoHome` and the catch-all drop the guard where they took it, and only the two arms that keep holding write a state back.

`HeldJ` joins the `pub use` in `state/mod.rs` and `lib.rs`.

## change 4: the root binds the timeout

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
/// The window elapsed with no `k`: flush the held `j` and reset.
pub(crate) fn jk_timeout(_ev: &JkTimeout, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    node.parent.typing_state.jk_state.flush()
}
```

`root.rs` adds `JkTimeout` to its imports. `jk_state` is non-`None` only while typing, so a `JkTimeout` after the hold resolved finds `None` and flushes nothing.

## change 5: tests

`crates/mercury/tests/transitions.rs`. The v0 cases hold, except `j` down now also returns the `Timer` effect; rebuild the expected `Timer` to assert it (its `testing` equality is the delay and the fire event).

```rust
let (_guard, effect) =
    freddie::timer_effect_and_guard(JK_TIMEOUT, MercuryEvent::JkTimeout(JkTimeout));
assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![MercuryEffect::Timer(effect)]));

// window elapses while j is still down: flush its down only.
assert_eq!(
    m.handle(&MercuryEvent::JkTimeout(JkTimeout)),
    Some(vec![emit(Key::KeyJ, PressType::Down, ModifierFlags::empty())]),
);

// a JkTimeout with nothing pending is a no-op.
let mut m = Mercury::default();
assert_eq!(m.handle(&MercuryEvent::JkTimeout(JkTimeout)), Some(vec![]));
```
