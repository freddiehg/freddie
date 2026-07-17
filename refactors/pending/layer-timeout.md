# layer idle timeout

A layer can time out: after a dwell with no input, it returns home. The `Layer` node binds `LayerTimeout` to a handler that goes home, so any layer inherits the behavior; a layer that wants a timeout holds a `TimerGuard` and arms one on entry. Leaving the layer drops its node and the guard, cancelling the timer. Nav is the first layer to use it, with a ten-second dwell.

This builds on `refactors/pending/timer-events.md` (the `freddie` timer primitive and the `MercuryEffect::Timer` effect). It adds the timeout event, the `Layer` binding, the guard field on `NavLayer`, and the entry that arms it.

## change 1: the layer timeout event and trigger

`crates/mercury/src/sources.rs`, appended:

```rust
/// A trigger matching a layer's idle-timeout firing.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LayerTimeout;

/// A layer's idle-timeout fired.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct LayerTimeoutEvent;

impl EventTrigger for LayerTimeout {
    type Event = LayerTimeoutEvent;
    fn is_matching(&self, _ev: &LayerTimeoutEvent) -> bool {
        true
    }
}
```

`crates/mercury/src/model.rs`, `MercuryTrigger`, before:

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyModifierKey(AnyModifierKey),
    AnyNonModifierKey(AnyNonModifierKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
}
```

after:

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyModifierKey(AnyModifierKey),
    AnyNonModifierKey(AnyNonModifierKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
    LayerTimeout(LayerTimeout),
}
```

`crates/mercury/src/model.rs`, `MercuryEvent`, before:

```rust
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
}
```

after:

```rust
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
    LayerTimeout(LayerTimeoutEvent),
}
```

The `use crate::{..}` in `model.rs` picks up `LayerTimeout` and `LayerTimeoutEvent`.

## change 2: the Layer node returns home on timeout

`crates/mercury/src/state.rs`, `Layer`, before:

```rust
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => to_home)]
pub enum Layer {
```

after:

```rust
#[derive(Bind, Debug, derive_more::From)]
#[node(parent = MercuryPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => to_home,
    LayerTimeout => on_layer_timeout,
)]
pub enum Layer {
```

`crates/mercury/src/handlers/home.rs`, appended (beside `to_home`, which the `Layer` node also binds):

```rust
use crate::LayerTimeoutEvent;
use crate::state::LayerPath;

/// A layer idled out: go home. Bound on the `Layer` node, so a fired timeout returns home from
/// whichever layer is active.
pub(crate) fn on_layer_timeout(
    _ev: &LayerTimeoutEvent,
    node: Node<LayerPath, ()>,
) -> Vec<MercuryEffect> {
    go_home(node.parent.ascend_to::<MercuryPath>())
}
```

## change 3: nav holds the guard and arms the timeout

`crates/mercury/src/state.rs`, `NavLayer`, before:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}
```

after:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {
    timeout: TimerGuard,
}
```

The `timeout` field is plain data, so the derive still treats `NavLayer` as a leaf.

`crates/mercury/src/state.rs` gains `use freddie::{timer_effect_and_guard, TimerGuard};` and `use std::time::Duration;`, the dwell constant, and the entry method that builds the pair where `NavLayer`'s private field is reachable, holds the guard, and returns the effect:

```rust
/// How long a layer sits idle before returning home.
const LAYER_TIMEOUT: Duration = Duration::from_secs(10);

impl Mercury {
    /// Enter the nav layer, arming its idle-timeout. Sitting in nav for `LAYER_TIMEOUT` fires
    /// `LayerTimeout`, which returns home; picking an app or leaving first drops the layer, and
    /// the guard with it, cancelling the timer.
    pub fn enter_nav(&mut self) -> Vec<MercuryEffect> {
        let (timeout, effect) =
            timer_effect_and_guard(LAYER_TIMEOUT, MercuryEvent::LayerTimeout(LayerTimeoutEvent));
        let mut effects = self.set_layer(NavLayer { timeout });
        effects.push(MercuryEffect::Timer(effect));
        effects
    }
}
```

`crates/mercury/src/handlers/home.rs`, `to_nav`, before:

```rust
pub(crate) fn to_nav<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().set_layer(NavLayer {})
}
```

after:

```rust
pub(crate) fn to_nav<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().enter_nav()
}
```

`home.rs`'s `use crate::state::{..}` drops `NavLayer` (now unused there).

## change 4: tests

`crates/mercury/tests/transitions.rs`, new cases through `SimpleRunner`. Entering nav returns a `Timer` effect; rebuild the expected effect to assert it (its `testing` equality is the delay and the fire event, so the receiver is ignored), then drive the fire by queueing `LayerTimeout`:

```rust
let (_guard, effect) =
    freddie::timer_effect_and_guard(LAYER_TIMEOUT, MercuryEvent::LayerTimeout(LayerTimeoutEvent));
assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![MercuryEffect::Timer(effect)]));
```

- `n` from home enters nav and asks for the timer; a queued `LayerTimeout` then lands in home.
- `n` then `c` foregrounds Chrome and enters the in-app layer; a `LayerTimeout` queued afterward returns to home from the in-app layer (the `Layer` node binds it whichever layer is active).
- `LayerTimeout` in home re-enters home.

Existing tests enter nav by dispatching `n` and match `Layer::Nav(_)` ignoring fields, so they still pass unchanged. `LAYER_TIMEOUT` is `pub(crate)` so the test can name it.
