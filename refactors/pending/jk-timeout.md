# jk's timeout

`jk` leaves typing for home, built on `KeySequence` in `freddie_keys`: an ordered run of keys, swallowed as they arrive, replayed in arrival order if the run breaks and dropped when it completes. mercury holds one, `[j, k]`, in `TypingState`, and `maybe_pass_through` feeds every typing key to it.

What is missing is a bound on how long a run can sit half-typed, and there is no bound at all today: a `j` swallowed now completes a run against a `k` pressed at any later moment, minutes later, because nothing but another key ever ends it.

Two things follow, and the second is the one that bites.

A `j` at the end of a line does not reach the app until the next keystroke does.

And the literal string `jk` cannot be typed. There is no gap long enough to separate the two keys, so writing about `jk` in a commit message or a doc means leaving the layer. A window makes the pause work: hold off past `JK_TIMEOUT` and the `j` types itself, leaving the `k` an ordinary `k`.

A run waits `JK_TIMEOUT` for its next key; on expiry what was swallowed replays and the run resets, so a later `k` is an ordinary `k`.

The guard lives INSIDE the run. A guard beside the sequence is two things that have to agree about whether a run is live, and nothing but a comment keeps them in step: every path that ends a run would have to remember to drop it. Owned by the run, it is dropped by the same code that clears `swallowed`, and cancelling on drop then falls out of the existing lifetime rather than being maintained.

`KeySequence` still does not name `freddie`'s types. It is generic over what a live run holds, since it cares only that the thing dies when the run does:

```rust
pub struct KeySequence<G = ()> {
    keys: &'static [Key],
    swallowed: Vec<KeyPress>,
    /// Held while the run is live and dropped with it. `()` for a run with nothing to hold;
    /// mercury's is a `TimerGuard`, whose `Drop` cancels the window.
    held: Option<G>,
}
```

mercury's is a `KeySequence<TimerGuard>`. `freddie_keys` gains no dependency, and `freddie` (which pulls tokio) stays out of the vocabulary crate.

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

/// Arm a run's window: the guard cancels it on drop, the effect schedules it. The delay is the
/// run's own, read off the sequence, so this does not restate the policy.
///
/// `pub(crate)` where `arm_return_home` is private, because the root's handlers call this one and
/// they are not children of this module.
pub(crate) fn arm_jk_timeout(window: Duration) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(window, MercuryEvent::JkTimeout(JkTimeout));
    (guard, MercuryEffect::Timer(effect))
}
```

The `use crate::{..}` in `state/mod.rs` adds `JkTimeout`; `JK_TIMEOUT` joins the `pub use` through `state/mod.rs` and `lib.rs`.

## change 3: the run owns its window and holds what dies with it

`crates/freddie_keys/src/sequence.rs`. Two changes to the same struct.

How long a run waits is part of what the run IS, so the constructor takes it: `Option<Duration>`, `None` for a run that never expires, which is every run today. The sequence does not arm anything itself — it has no event type and no timer — but it is what the caller asks, so the policy lives with the sequence instead of beside it. It is also what a later lazy expiry would read directly, checking the window against the incoming key's time rather than needing a timer at all.

And it gains a type parameter for whatever a live run holds, defaulting to `()` so a run with nothing to hold is unchanged, dropping it wherever it clears `swallowed`.

before:

```rust
pub struct KeySequence {
    keys: &'static [Key],
    swallowed: Vec<KeyPress>,
}
```

after:

```rust
pub struct KeySequence<G = ()> {
    keys: &'static [Key],
    /// How long the run waits for its next key, or `None` for one that waits forever. The run
    /// does not enforce it; the caller reads it to arm whatever wakes the run up.
    window: Option<Duration>,
    swallowed: Vec<KeyPress>,
    /// Held while the run is live and dropped with it, so a caller cannot leave it behind. `()`
    /// for a run with nothing to hold; mercury's is a `TimerGuard`, whose `Drop` cancels the
    /// window.
    held: Option<G>,
}
```

`new` takes the window and sets `held: None`, before:

```rust
    pub const fn new(keys: &'static [Key]) -> Self {
        assert!(!keys.is_empty(), "a sequence needs at least one key");
        Self {
            keys,
            swallowed: Vec::new(),
        }
    }
```

after:

```rust
    pub const fn new(keys: &'static [Key], window: Option<Duration>) -> Self {
        assert!(!keys.is_empty(), "a sequence needs at least one key");
        Self {
            keys,
            window,
            swallowed: Vec::new(),
            held: None,
        }
    }

    /// How long this run waits for its next key, or `None` if it waits forever.
    #[must_use]
    pub const fn window(&self) -> Option<Duration> {
        self.window
    }
```

`sequence.rs` imports `std::time::Duration`, which is std, so `freddie_keys` gains no dependency. The two places that end a run clear `held`. `interrupt`, before:

```rust
    pub fn interrupt(&mut self) -> Vec<KeyPress> {
        std::mem::take(&mut self.swallowed)
    }
```

after:

```rust
    pub fn interrupt(&mut self) -> Vec<KeyPress> {
        self.held = None;
        std::mem::take(&mut self.swallowed)
    }
```

and the completing arm of `advance`, before:

```rust
                if matched + 1 == self.keys.len() {
                    self.swallowed.clear();
                    KeySequenceOutcome::Completed
```

after:

```rust
                if matched + 1 == self.keys.len() {
                    self.swallowed.clear();
                    self.held = None;
                    KeySequenceOutcome::Completed
```

The caller hands over what to hold when a run opens:

```rust
    /// Give the live run something to own until it ends: a timer guard, typically, so the window
    /// is cancelled by the run ending rather than by the caller remembering to.
    ///
    /// # Panics
    ///
    /// If the run is idle. Nothing would ever drop it.
    pub fn hold(&mut self, guard: G) {
        assert!(!self.is_idle(), "an idle run has no life to tie a guard to");
        self.held = Some(guard);
    }
```

`TypingState`'s field becomes `pub jk: KeySequence<TimerGuard>`, and `state/mod.rs` already imports `TimerGuard`. Both `KeySequence::new(JK)` call sites there become `KeySequence::new(JK, Some(JK_TIMEOUT))`, as do the ones in `crates/freddie_keys/tests/sequence.rs`, which pass `None`: those cases assert the machine, and none of them involves a window.

## change 4: the handler arms and disarms

`crates/mercury/src/handlers/root.rs`, `maybe_pass_through`. The run is idle before the key iff this key opens it, so that is when the timer is armed and handed over. Every other outcome ends the run, which drops the guard and cancels the window with no bookkeeping here.

before:

```rust
    match root.typing_state.jk.advance(ev) {
        KeySequenceOutcome::Advanced => Vec::new(),
```

after:

```rust
    let opening = root.typing_state.jk.is_idle();
    match root.typing_state.jk.advance(ev) {
        // Opening a run arms its window, if it has one. A run with no window needs no wake-up.
        KeySequenceOutcome::Advanced if opening => match root.typing_state.jk.window() {
            Some(window) => {
                let (guard, timer) = arm_jk_timeout(window);
                root.typing_state.jk.hold(guard);
                vec![timer]
            }
            None => Vec::new(),
        },
        KeySequenceOutcome::Advanced => Vec::new(),
```

The other two arms are unchanged: both end the run, and the run drops the guard itself.

```rust
        KeySequenceOutcome::Passed(presses) => {
            let mut out = replay(presses);
            out.push(emit(ev.key, ev.press, ev.flags));
            out
        }
        KeySequenceOutcome::Completed => root.set_layer(HomeLayer::new()),
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
    replay(node.parent.typing_state.jk.interrupt())
}
```

`root.rs` adds `JkTimeout` to its imports. The guard is dropped whenever the run ends, so a `JkTimeout` for a run that already ended never fires; one that arrives anyway finds an idle run and emits nothing.

## change 6: tests

`crates/mercury/tests/transitions.rs`. The cases that landed with `jk` hold, except that the key which opens the run now also returns the `Timer` effect; rebuild the expected `Timer` to assert it (its `testing` equality is the delay and the fire event).

`emit(key, press)` is the existing helper and stamps no flags; `typing()` is the one that starts in the passthrough layer.

```rust
// The armed timer, rebuilt to assert against.
fn jk_timer() -> MercuryEffect {
    let (_guard, effect) =
        freddie::timer_effect_and_guard(JK_TIMEOUT, MercuryEvent::JkTimeout(JkTimeout));
    MercuryEffect::Timer(effect)
}

#[test]
fn a_half_typed_run_types_itself_when_the_window_elapses() {
    let mut m = typing();
    assert_eq!(m.handle(&key(Key::KeyJ)), Some(vec![jk_timer()]));
    assert_eq!(
        m.handle(&MercuryEvent::JkTimeout(JkTimeout)),
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
    assert_eq!(m.handle(&MercuryEvent::JkTimeout(JkTimeout)), Some(vec![]));
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
