# self-trigger macro

A payload-less source is its own event and always matches. `bind` gets a `macro_rules!` for that `EventTrigger` impl, and `Quit`, whose event carries nothing, collapses to one type through it.

## change 1: the macro

`crates/bind/src/lib.rs`, appended:

```rust
/// Implements [`EventTrigger`] for a payload-less trigger that is its own event and always
/// matches, for a bare signal that carries nothing and has nothing to discriminate.
#[macro_export]
macro_rules! self_trigger {
    ($t:ty) => {
        impl $crate::EventTrigger for $t {
            type Event = Self;
            fn is_matching(&self, _event: &Self) -> bool {
                true
            }
        }
    };
}
```

## change 2: Quit is its own event

`crates/mercury/src/sources.rs`, `Quit` and `QuitEvent`, before:

```rust
/// A trigger that matches a quit request, wherever it came from (the menu bar for
/// now). It carries no key: it is a single, layer-independent "quit now".
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quit;

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct QuitEvent;

impl EventTrigger for Quit {
    type Event = QuitEvent;
    fn is_matching(&self, _ev: &QuitEvent) -> bool {
        true
    }
}
```

after:

```rust
/// A quit request, wherever it came from (the menu bar for now). It carries no key: it is a
/// single, layer-independent "quit now", so one type is both the trigger and the event.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quit;

bind::self_trigger!(Quit);
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
    Quit(Quit),
}
```

`model.rs`'s `use crate::{..}` drops `QuitEvent` (`Quit` is already imported for `MercuryTrigger`).

`crates/mercury/src/state.rs`, `quit_event`, before:

```rust
#[must_use]
pub const fn quit_event() -> MercuryEvent {
    MercuryEvent::Quit(QuitEvent)
}
```

after:

```rust
#[must_use]
pub const fn quit_event() -> MercuryEvent {
    MercuryEvent::Quit(Quit)
}
```

`state.rs`'s `use crate::{..}` drops `QuitEvent` and keeps `Quit`.

`crates/mercury/src/handlers/quit.rs`, `on_quit`, before:

```rust
pub(crate) fn on_quit(_ev: &QuitEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
```

after:

```rust
pub(crate) fn on_quit(_ev: &Quit, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
```

`quit.rs`'s `use crate::{MercuryEffect, QuitEvent};` becomes `use crate::{MercuryEffect, Quit};`.

`crates/mercury/src/lib.rs`, the `sources` re-export, before:

```rust
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, Quit, QuitEvent,
};
```

after:

```rust
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, Quit,
};
```
