# one timer event, many timers

Every timer mercury owns currently costs a type of its own: `LayerTimeout` is a struct, a `MercuryTrigger` variant, a `MercuryEvent` variant, and a `self_trigger!` impl, all to say "the layer's idle timer went off". A second timer would cost the same again, and a third after that.

There is nothing per-timer about any of it except which timer fired. So there is one event, `TimerFired`, carrying the identity of the timer that fired, and one trigger, `Timer(TimerId)`, that matches its own id. Adding a timer becomes adding a `TimerId` variant.

The id is an enum rather than a generated handle, and it has to be. A binding is written in the `#[bind(..)]` attribute at compile time, so the trigger value has to be nameable there: `Timer(TimerId::ReturnHome)` names it, and a uuid minted when the timer is armed cannot. The trigger is parsed as a `syn::Expr` (`bind_macro`'s `Binding`), so a call expression is accepted where a bare path is today.

Two bindings with different ids are different `MercuryTrigger` values, so the duplicate-trigger check still tells them apart, and one node binding `Timer(ReturnHome)` while another binds `Timer(JkWindow)` is not a collision.

`freddie`'s timer machinery does not change at all. `timer_effect_and_guard` is generic over the event it fires, so this is entirely mercury's vocabulary.

## change 1: the id, the event, and the trigger

`crates/mercury/src/sources.rs`, before:

```rust
/// A layer's idle-timeout. It carries nothing, so one type is both the trigger and the event it
/// fires.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LayerTimeout;

bind::self_trigger!(LayerTimeout);
```

after:

```rust
/// Which timer fired.
///
/// One variant per timer mercury can arm. A new timer is a variant here and a binding for it, not
/// a new event type: what a timer firing means is entirely "this one went off".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TimerId {
    /// A chooser layer idling back home. Armed by `arm_return_home`.
    ReturnHome,
}

/// A timer fired. Carries which one, so every timer shares this event.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerFired {
    pub id: TimerId,
}

/// A trigger matching one timer's firing, by id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Timer(pub TimerId);

impl EventTrigger for Timer {
    type Event = TimerFired;
    fn is_matching(&self, ev: &TimerFired) -> bool {
        self.0 == ev.id
    }
}
```

`sources.rs` already imports `EventTrigger`.

## change 2: the model's trigger and event

`crates/mercury/src/model.rs`, `MercuryTrigger`, before:

```rust
    LayerTimeout(LayerTimeout),
```

after:

```rust
    Timer(Timer),
```

`MercuryEvent`, before:

```rust
    LayerTimeout(LayerTimeout),
```

after:

```rust
    Timer(TimerFired),
```

Its `use crate::{..}` swaps `LayerTimeout` for `Timer, TimerFired`. `lib.rs` re-exports `Timer`, `TimerFired`, and `TimerId` from `sources` in place of `LayerTimeout`.

## change 3: arming and binding it

`crates/mercury/src/state/mod.rs`, `arm_return_home`, before:

```rust
/// Arm the return-to-home timer a layer holds: the guard cancels it on drop, and the effect
/// schedules it. It fires [`LayerTimeout`] after [`RETURN_TO_HOME_TIMEOUT`], which the `Layer`
/// node binds home.
fn arm_return_home() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(
        RETURN_TO_HOME_TIMEOUT,
        MercuryEvent::LayerTimeout(LayerTimeout),
    );
    (guard, MercuryEffect::Timer(effect))
}
```

after:

```rust
/// Arm the return-to-home timer a layer holds: the guard cancels it on drop, and the effect
/// schedules it. It fires [`TimerId::ReturnHome`] after [`RETURN_TO_HOME_TIMEOUT`], which the
/// `Layer` node binds home.
fn arm_return_home() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(
        RETURN_TO_HOME_TIMEOUT,
        MercuryEvent::Timer(TimerFired {
            id: TimerId::ReturnHome,
        }),
    );
    (guard, MercuryEffect::Timer(effect))
}
```

`Layer`'s `#[bind(..)]`, before:

```rust
#[bind(LayerTimeout => to_home)]
```

after:

```rust
#[bind(Timer(TimerId::ReturnHome) => to_home)]
```

`to_home` is already generic over the event it is handed, so it takes `&TimerFired` with no change. The `use crate::{..}` in `state/mod.rs` swaps `LayerTimeout` for `Timer, TimerFired, TimerId`.

## change 4: tests

`crates/mercury/tests/transitions.rs`. Every `MercuryEvent::LayerTimeout(LayerTimeout)` becomes:

```rust
MercuryEvent::Timer(TimerFired {
    id: TimerId::ReturnHome,
})
```

which is worth a helper beside `return_home_timer`, since it appears in the rebuilt timer effect and in the three tests that drive the firing:

```rust
// The event the return-home timer fires.
const fn return_home_fired() -> MercuryEvent {
    MercuryEvent::Timer(TimerFired {
        id: TimerId::ReturnHome,
    })
}
```

Add one case that the id is what discriminates, so a second timer's firing cannot be mistaken for this one. It needs a second variant to test against, so it lands with the first timer that adds one rather than here.
