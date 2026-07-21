---
title: Testing
sidebar_position: 8
---

# Testing

`state.handle` takes state and event and returns the updated state and the effects. It performs none of them, so a test is a function call and an `assert_eq!`. There is no keyboard to drive, no daemon to start, and nothing to mock.

```rust
#[test]
fn home_n_enters_nav() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        Some(vec![shows("Nav"), return_home_timer()])
    );
    assert!(matches!(m.layer(), Layer::Nav(_)));
}
```

Three things are asserted, and the third is the one people forget:

- The event was handled at all. `handle` returns an `Option`, and `None` means no binding claimed it.
- The exact effects, in order. Not that a `ShowLayer` appeared somewhere, but that these two came back and nothing else did.
- The resulting state. `n` from home leaves you in nav, and a binding that produced the right effects while landing in the wrong layer is still wrong.

## Where a test starts

Tests build the state they mean to exercise rather than pressing keys to get there:

```rust
// A mercury in Home, the command layer. The default is Typing
// (passthrough), but most per-event tests exercise Home's
// command bindings, so they start here.
fn home() -> Mercury {
    Mercury::with_layer(Layer::Home(HomeLayer {}))
}
```

A test that walks in from the typing layer is testing the walk as well as the binding, and fails for two reasons instead of one.

## Timers

A timer effect mints its own id, so a test cannot write the id it expects. It rebuilds the effect and compares:

```rust
fn return_home_timer() -> MercuryEffect {
    let (_guard, effect) =
        freddie::timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, fired);
    MercuryEffect::Timer(effect)
}
```

Under the `testing` feature, equality on a timer effect compares the delay and the event it will fire, not the id. To assert on a firing, read the id back off the effect that set it, since nothing else can know it.

## Driving the loop

The per-event tests call `handle` directly. When a test needs several events to feed each other, it drives a `bind::SimpleRunner`, which records the effects and, for a `Foreground` effect, reports the app back the way the OS watcher would.

That is the difference between the two kinds of test in `crates/mercury/tests/transitions.rs`: one asserts what a single event produces, the other asserts what a sequence settles into.

## The standard

The standard for the model is exhaustive: every key in every reachable state, asserting exactly what dispatch produces. Because the model is a pure function of state and event, the full table is checkable, and it doubles as documentation of the keymap.

`transitions.rs` holds 87 of these today, which is not the whole table. A new binding extends toward it rather than testing only the happy path.

A table test is the cheapest way to add a layer's worth at once:

```rust
#[test]
fn resize_answers_exactly_its_keymap() {
    for (k, effects) in [
        (Key::UpArrow, vec![place(Placement::Maximize)]),
        (Key::LeftArrow, vec![place(Placement::LeftHalf)]),
        (Key::RightArrow, vec![place(Placement::RightHalf)]),
    ] {
        let mut m = home();
        let _ = m.handle(&key(Key::KeyR));
        assert_eq!(m.handle(&key(k)), Some(leaves(effects)), "{k:?}");
    }
}
```
