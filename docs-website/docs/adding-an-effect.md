---
title: Adding an Effect
sidebar_position: 5
---

# Adding an Effect

A handler returns effects. It does not perform them. That separation is what keeps `state.handle` pure, and therefore testable.

```rust
let effects = state.handle(event).unwrap_or_default();
for effect in effects {
    // perform it
}
```

## What a handler hands back

`Bindings::Output` is what dispatch returns, and for `mercury` it is `Vec<MercuryEffect>`. A handler is not held to that type. It returns anything that is `Into` the output, so a handler with one effect to ask for returns it bare:

```rust
pub(crate) const fn refresh<E, N>(_ev: &E, _node: N) -> MercuryEffect {
    tap(Key::KeyR, ModifierFlags::COMMAND)
}
```

That compiles because `mercury` owns the conversion:

```rust
impl From<MercuryEffect> for Vec<MercuryEffect> {
    fn from(effect: MercuryEffect) -> Self {
        vec![effect]
    }
}
```

The framework asks only for `Into<Self::Output>`, so which return types are legal is a decision each program makes by writing the impls it wants.

## Adding a variant

TODO: add the variant to `MercuryEffect`, and show the shape an effect carrying data takes.

## Handling it

TODO: show where the effect loop lives and how a new arm reaches the platform crate that performs it.

## Effects that need the main thread

TODO: which effects have to run on the main thread on macOS, and how they get there.

## Timer effects

`state.handle` can create timer effects, which bump a global ID; dropping the corresponding timer guard prevents those from firing. Because tests do not execute effects, `state.handle` stays pure and testable, and tests assert nothing about timer IDs.

TODO: show setting a timer from a handler and holding its guard on the level that owns it.

## Testing

TODO: assert on the returned effects rather than on anything performed.
