# jk leaves typing for home

In the typing layer, `j` then `k` returns to home; neither key reaches the app. A `j` not followed by `k` types itself. The passthrough state (the held modifiers and the pending `j`) lives in the root's `TypingState`; `TypingState::process` runs the sequence. v0 adds the `jk` state machine, v1 a timeout.

Holding a `j` swallows its down and its up, so the sequence keeps them and a flush replays them exactly. `JDown` holds the down alone, replayed while `j` is still physically down. `JUp` holds both, replayed as a full tap. A held modifier suspends the sequence: with any modifier down, `j` passes straight through.

# v0: the jk sequence

## change 1: the event newtypes, JkState, and the jk_state field

`crates/mercury/src/state/mod.rs`, new. `DownEvent`/`UpEvent` wrap a `KeyEvent` whose direction is fixed, built only in the matched arms of `advance`.

```rust
/// A key event known to be a press-down, kept so the held key replays exactly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DownEvent(KeyEvent);

/// A key event known to be a release.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UpEvent(KeyEvent);

impl DownEvent {
    fn emit(&self) -> MercuryEffect {
        MercuryEffect::Emit(self.0.clone())
    }
}

impl UpEvent {
    fn emit(&self) -> MercuryEffect {
        MercuryEffect::Emit(self.0.clone())
    }
}

/// Progress through the `jk` escape sequence, keeping the held `j`'s events.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum JkState {
    #[default]
    None,
    /// `j` down was swallowed; `j` is still physically down.
    JDown(DownEvent),
    /// `j` down and up were both swallowed; `j` is physically up.
    JUp((DownEvent, UpEvent)),
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

`state/mod.rs` and `lib.rs` add `DownEvent`, `UpEvent`, `JkState` to their `pub use`.

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
    /// Replay the held `j` and reset: its down while `j` is still physically down, both once its up
    /// has been swallowed.
    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        let out = match self {
            Self::None => Vec::new(),
            Self::JDown(down) => vec![down.emit()],
            Self::JUp((down, up)) => vec![down.emit(), up.emit()],
        };
        *self = Self::None;
        out
    }

    /// Advance the sequence for one key (no modifier held).
    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (&*self, ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                *self = Self::JDown(DownEvent(ev.clone()));
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(_), Key::KeyJ, PressType::Up) => {
                if let Self::JDown(down) = std::mem::replace(self, Self::None) {
                    *self = Self::JUp((down, UpEvent(ev.clone())));
                }
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

`crates/mercury/src/state/mod.rs`, `JkState`, before:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub enum JkState {
    #[default]
    None,
    JDown(DownEvent),
    JUp((DownEvent, UpEvent)),
}
```

after:

```rust
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, Default)]
pub enum JkState {
    #[default]
    None,
    /// `j` down was swallowed; `j` is still physically down. The guard arms the timeout.
    JDown(DownEvent, TimerGuard),
    /// `j` down and up were both swallowed; `j` is physically up. The guard carries over from
    /// `JDown`, so the window runs from the first `j`.
    JUp((DownEvent, UpEvent), TimerGuard),
}
```

`flush` matches the extra field; `advance` arms on the first `j` and carries the guard, dropping it on the paths that resolve the hold. `flush` and `advance`, before:

```rust
    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        let out = match self {
            Self::None => Vec::new(),
            Self::JDown(down) => vec![down.emit()],
            Self::JUp((down, up)) => vec![down.emit(), up.emit()],
        };
        *self = Self::None;
        out
    }

    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (&*self, ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                *self = Self::JDown(DownEvent(ev.clone()));
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(_), Key::KeyJ, PressType::Up) => {
                if let Self::JDown(down) = std::mem::replace(self, Self::None) {
                    *self = Self::JUp((down, UpEvent(ev.clone())));
                }
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
    pub(crate) fn flush(&mut self) -> Vec<MercuryEffect> {
        let out = match self {
            Self::None => Vec::new(),
            Self::JDown(down, _) => vec![down.emit()],
            Self::JUp((down, up), _) => vec![down.emit(), up.emit()],
        };
        *self = Self::None;
        out
    }

    pub(crate) fn advance(&mut self, ev: &KeyEvent) -> JkOutcome {
        match (&*self, ev.key, ev.press) {
            (Self::None, Key::KeyJ, PressType::Down) => {
                let (guard, timer) = arm_jk_timeout();
                *self = Self::JDown(DownEvent(ev.clone()), guard);
                JkOutcome::Emit(vec![timer])
            }
            (Self::JDown(..), Key::KeyJ, PressType::Up) => {
                if let Self::JDown(down, guard) = std::mem::replace(self, Self::None) {
                    *self = Self::JUp((down, UpEvent(ev.clone())), guard);
                }
                JkOutcome::Emit(Vec::new())
            }
            (Self::JDown(..), Key::KeyJ, PressType::Down) => JkOutcome::Emit(Vec::new()),
            (Self::JDown(..) | Self::JUp(..), Key::KeyK, PressType::Down) => JkOutcome::GoHome,
            (Self::JDown(..) | Self::JUp(..), ..) => {
                let mut out = self.flush();
                out.push(emit(ev.key, ev.press, ev.flags));
                JkOutcome::Emit(out)
            }
            (Self::None, ..) => JkOutcome::Emit(vec![emit(ev.key, ev.press, ev.flags)]),
        }
    }
```

`flush` in the catch-all drops the guard as it resets; `GoHome` drops it when `set_layer` clears `jk_state`.

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
