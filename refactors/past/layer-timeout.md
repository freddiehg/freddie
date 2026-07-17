# layer idle timeout

A layer can time out: after a dwell with no input, it returns home. The `Layer` node binds `LayerTimeout` to a handler that goes home, so any layer inherits the behavior; a layer that wants a timeout holds a `TimerGuard` and arms one on entry. Leaving the layer drops its node and the guard, cancelling the timer. Nav is the first layer to use it, with a ten-second dwell.

This builds on `refactors/pending/timer-events.md` (the timer primitive and the `Timer` effect), `refactors/pending/layer-modules.md` (each layer built through `new`), and `refactors/pending/self-trigger.md` (the `self_trigger!` macro). It adds the timeout event, the `Layer` binding, and the guard on `NavLayer`.

## change 1: the layer timeout trigger

`crates/mercury/src/sources.rs`, appended:

```rust
/// A layer's idle-timeout. It carries nothing, so one type is both the trigger and the event it
/// fires.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LayerTimeout;

bind::self_trigger!(LayerTimeout);
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
    LayerTimeout(LayerTimeout),
}
```

`crates/mercury/src/lib.rs`, the `sources` re-export, before:

```rust
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, Quit, QuitEvent,
};
```

after:

```rust
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, LayerTimeout, Quit,
    QuitEvent,
};
```

`model.rs`'s `use crate::{..}` then picks up `LayerTimeout`.

## change 2: the Layer node returns home on timeout

`to_home` already goes home and ignores its event, so generalize it over the event type and bind `LayerTimeout` to it alongside `escape`.

`crates/mercury/src/handlers/home.rs`, `to_home`, before:

```rust
pub(crate) fn to_home<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    go_home(node.parent.ascend())
}
```

after (generic over the fired event, so any trigger can bind it):

```rust
pub(crate) fn to_home<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    go_home(node.parent.ascend())
}
```

`crates/mercury/src/state/mod.rs`, `Layer`, before:

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
    LayerTimeout => to_home,
)]
pub enum Layer {
```

## change 3: nav holds the guard and arms the timeout

`NavLayer::new` (from `layer-modules.md`) gains the timeout: it arms the timer, keeps the guard, and returns the effect that schedules it.

`crates/mercury/src/state/nav.rs`, before:

```rust
use bind::Bind;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::MercuryStruct;
use super::LayerPath;

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

impl NavLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {}
    }
}
```

after:

```rust
use std::time::Duration;

use bind::Bind;
use freddie::{timer_effect_and_guard, TimerGuard};
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{LayerTimeout, MercuryEffect, MercuryEvent, MercuryStruct};
use super::LayerPath;

/// How long nav sits idle before returning home.
pub const RETURN_TO_HOME_TIMEOUT: Duration = Duration::from_secs(10);

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

impl NavLayer {
    /// Build the nav layer with its idle-timeout armed, returning the layer and the effect that
    /// schedules the timeout.
    #[must_use]
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, effect) =
            timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, MercuryEvent::LayerTimeout(LayerTimeout));
        (Self { timeout }, MercuryEffect::Timer(effect))
    }
}
```

The `timeout` field is plain data, so the derive still treats `NavLayer` as a leaf.

`state/mod.rs` re-exports the constant with the layer (`pub use nav::{NavLayer, RETURN_TO_HOME_TIMEOUT};`), and `lib.rs` adds `RETURN_TO_HOME_TIMEOUT` to its `pub use state::{..}`, so the test can name `RETURN_TO_HOME_TIMEOUT`.

`crates/mercury/src/handlers/home.rs`, `to_nav` (which `layer-modules.md` left calling `NavLayer::new`), unpacks the pair: `set_layer(nav)`, then folds the timer effect into the result. Before:

```rust
pub(crate) fn to_nav<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().set_layer(NavLayer::new())
}
```

after:

```rust
pub(crate) fn to_nav<'a, P: Ascend<MercuryPath<'a>>>(
    _ev: &KeyEvent,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend().set_layer(nav);
    effects.push(timer);
    effects
}
```

## change 4: tests

`crates/mercury/tests/transitions.rs`, new cases through `SimpleRunner`. Entering nav returns a `Timer` effect; rebuild the expected effect to assert it (its `testing` equality is the delay and the fire event, so the receiver is ignored), then drive the fire by queueing `LayerTimeout`:

```rust
let (_guard, effect) =
    freddie::timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, MercuryEvent::LayerTimeout(LayerTimeout));
assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![MercuryEffect::Timer(effect)]));
```

- `n` from home enters nav and asks for the timer; a queued `LayerTimeout` then lands in home.
- `n` then `c` foregrounds Chrome and enters the in-app layer; a `LayerTimeout` queued afterward returns to home from the in-app layer (the `Layer` node binds it whichever layer is active).
- `LayerTimeout` in home re-enters home.

Existing tests enter nav by dispatching `n` and match `Layer::Nav(_)` ignoring fields, so they still pass unchanged.
