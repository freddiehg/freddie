# jk's timeout

`jk` leaves typing for home, built on `KeySequence` in `freddie_keys`: an ordered run of keys, swallowed as they arrive, replayed in arrival order if the run breaks and dropped when it completes. mercury holds one, `[j, k]`, in `TypingState`, and `maybe_pass_through` feeds every typing key to it.

What is missing is a bound on how long a run can sit half-typed. A `j` with no `k` after it stays swallowed until some other key breaks the run, so a `j` typed at the end of a line does not reach the app until the next keystroke does.

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
    replay(root.typing_state.jk.interrupt())
}
```

`root.rs` adds `JkTimeout` to its imports. The guard is dropped whenever the run ends, so a `JkTimeout` for a run that already ended never fires; one that arrives anyway finds an idle run and emits nothing.

## change 6: tests

`crates/mercury/tests/transitions.rs`. The cases that landed with `jk` hold, except that the key which opens the run now also returns the `Timer` effect; rebuild the expected `Timer` to assert it (its `testing` equality is the delay and the fire event).

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
