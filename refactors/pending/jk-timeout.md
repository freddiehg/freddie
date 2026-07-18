# jk's timeout

`jk` leaves typing for home, built on `KeySequence` in `freddie_keys`: an ordered run of keys, swallowed as they arrive, replayed in arrival order if the run breaks and dropped when it completes. mercury holds one, `[j, k]`, in `TypingState`, and `maybe_pass_through` feeds every typing key to it.

What is missing is a bound on how long a run can sit half-typed, and there is no bound at all today: a `j` swallowed now completes a run against a `k` pressed at any later moment, minutes later, because nothing but another key ever ends it.

Two things follow, and the second is the one that bites.

A `j` at the end of a line does not reach the app until the next keystroke does.

And the literal string `jk` cannot be typed. There is no gap long enough to separate the two keys, so writing about `jk` in a commit message or a doc means leaving the layer. A window makes the pause work: hold off past `JK_TIMEOUT` and the `j` types itself, leaving the `k` an ordinary `k`.

A run waits `JK_TIMEOUT` for its next key; on expiry what was swallowed replays and the run resets, so a later `k` is an ordinary `k`. The guard lives in mercury next to the sequence rather than inside it, so `freddie_keys` stays free of `freddie`'s timer types.

Builds on `refactors/pending/timer-ids.md`, which has to land first: it makes a timer's firing one event carrying which timer it was, so this adds a `TimerId` variant instead of a bespoke trigger. Also `refactors/past/timer-events.md` and `refactors/past/layer-timeout.md`.

## change 1: the timer id and the arm helper

Depends on `timer-ids.md`: one `TimerFired` event carries which timer went off, so this adds a variant rather than a trigger, an event, and a `self_trigger!` impl of its own.

`crates/mercury/src/sources.rs`, `TimerId`, before:

```rust
pub enum TimerId {
    /// A chooser layer idling back home. Armed by `arm_return_home`.
    ReturnHome,
}
```

after:

```rust
pub enum TimerId {
    /// A chooser layer idling back home. Armed by `arm_return_home`.
    ReturnHome,
    /// A half-typed key sequence's window. Armed by `arm_jk_timeout`.
    JkWindow,
}
```

## change 2: the duration and the arm helper

`crates/mercury/src/state/mod.rs`, by `RETURN_TO_HOME_TIMEOUT` and `arm_return_home`:

```rust
/// How long a run waits for its next key before what it swallowed types itself.
pub const JK_TIMEOUT: Duration = Duration::from_millis(500);

/// Arm the `jk` timeout: the guard cancels it on drop, the effect schedules it.
///
/// `pub(crate)` where `arm_return_home` is private, because the root's handlers call this one and
/// they are not children of this module.
pub(crate) fn arm_jk_timeout() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(
        JK_TIMEOUT,
        MercuryEvent::Timer(TimerFired {
            id: TimerId::JkWindow,
        }),
    );
    (guard, MercuryEffect::Timer(effect))
}
```

`JK_TIMEOUT` joins the `pub use` through `state/mod.rs` and `lib.rs`.

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
    /// Live exactly while `jk` is mid-run: dropping it cancels the timeout. Written by the two
    /// root handlers only, each in the same breath as the run it belongs to, so the two cannot
    /// disagree. `pub(crate)` because those handlers are not in this module.
    pub(crate) jk_timer: Option<TimerGuard>,
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
        KeySequenceOutcome::Passed(presses) => {
            root.typing_state.jk_timer = None;
            let mut out = replay(presses);
            out.push(emit(ev.key, ev.press, ev.flags));
            out
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
    Timer(TimerId::JkWindow) => jk_timeout,
    AnyKey => maybe_pass_through,
```

The `Layer` node binds `Timer(TimerId::ReturnHome)`, a different trigger value, so the two never collide however the layers nest.

`crates/mercury/src/handlers/root.rs`, new handler:

```rust
/// The window elapsed with no next key: what the run swallowed types itself.
pub(crate) fn jk_timeout(_ev: &TimerFired, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.typing_state.jk_timer = None;
    replay(root.typing_state.jk.interrupt())
}
```

`root.rs` adds `TimerFired` to its imports. The guard is dropped whenever the run ends, so a `JkWindow` firing for a run that already ended never arrives; one that arrives anyway finds an idle run and emits nothing.

## change 6: tests

`crates/mercury/tests/transitions.rs`. The cases that landed with `jk` hold, except that the key which opens the run now also returns the `Timer` effect; rebuild the expected `Timer` to assert it (its `testing` equality is the delay and the fire event).

`emit(key, press)` is the existing helper and stamps no flags; `typing()` is the one that starts in the passthrough layer.

```rust
// The event the jk window fires, and the armed timer rebuilt to assert against.
const fn jk_window_fired() -> MercuryEvent {
    MercuryEvent::Timer(TimerFired {
        id: TimerId::JkWindow,
    })
}

fn jk_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(JK_TIMEOUT, jk_window_fired());
    MercuryEffect::Timer(effect)
}

#[test]
fn a_half_typed_run_types_itself_when_the_window_elapses() {
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![jk_timer()]));
    assert_eq!(
        m.handle(&jk_window_fired()),
        Some(vec![emit(Key::KeyJ, PressType::Down)]),
    );
    assert!(m.typing_state.jk.is_idle());
    // The k that follows is an ordinary k, not the second half of anything.
    assert_eq!(m.handle(&key(Key::KeyK)), Some(passed(Key::KeyK)));
    assert!(matches!(m.layer(), Layer::Typing(_)));
}

#[test]
fn a_timeout_with_no_run_in_progress_emits_nothing() {
    let mut m = typing();
    assert_eq!(m.handle(&jk_window_fired()), Some(vec![]));
}

#[test]
fn the_window_runs_from_the_first_key_not_the_last() {
    // The j up advances the run without re-arming, so only the j down returns a timer.
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![jk_timer()]));
    assert_eq!(m.handle(&up(Key::KeyJ)), Some(vec![]));
}
```

Every case that landed with `jk` and opens a run needs its expected effects to gain `jk_timer()`, since the opening key now returns it.
