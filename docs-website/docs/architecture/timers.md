---
title: Timers
sidebar_position: 7
---

# Timers

A timer is two linked halves built by one call: a guard the owning node holds, and an effect a handler returns.

```rust
let (guard, effect) = freddie::timer_effect_and_guard(delay, fired);
```

The effect loop schedules the effect. Dropping the guard cancels it. Nothing else has to remember the timer exists.

## Who holds the guard

The level that wants the timer holds it as a field. `mercury`'s command layers each idle back home, so each keeps the guard for its own return-home timer:

```rust
pub struct NavLayer {
    home_timeout: TimerGuard,
}
```

That field is never read as data. It is held for its `Drop`, and it is read for its trigger. Both matter:

- Leaving the layer drops the layer, which drops the guard, which cancels the timer. There is no cancellation code anywhere, and no way to leave a layer and forget.
- While the layer is alive, the guard is what identifies the firing that belongs to it.

## Telling two timers apart

Every timer fires the same event:

```rust
Timer(TimerFired),
```

So the event alone cannot say which one went off. What distinguishes them is which node is still holding that guard, and the binding says so with a closure over its own state:

```rust
#[bind(
    // Only this layer's own timer: a firing from a layer
    // you already left matches nothing.
    |path| path.get().home_timeout.trigger() => to_home,
)]
```

`trigger()` returns a trigger matching that timer's id and no other. A firing from a layer you have left matches nothing, so it is dropped rather than misrouted, and the layer that armed it is already gone.

The closure is handed a shared reference, so a trigger reads state and cannot write it.

## Ids

An id is minted when the timer is set and stamped on both halves. It is process-wide and monotonic, so no two timers share one, whoever set them.

Nothing outside the timer can know an id in advance. That is the property tests rely on: a test rebuilds the effect and compares, because equality under the `testing` feature looks at the delay and the event rather than the id. To assert on a firing, read the id back off the effect that set it. [Testing](./testing.md) covers the pattern.

## Purity

`state.handle` can create timer effects, and creating one bumps a global counter. That is the one impure thing in dispatch, and it is why the qualification exists: because tests do not execute effects, including timer effects, `handle` stays a pure function of state and event, and tests assert nothing about ids.

## Resetting one

Activity in a layer that should extend its own timeout drops the old guard and arms a new one. Dropping first is what cancels the timer already pending, so the two never race:

```rust
let (timeout, timer) = arm_return_home();
self.home_timeout = timeout;   // drops the old guard
```

The returned effect goes out with whatever else the handler produced.
