---
title: Adding an Effect
sidebar_position: 5
---

# Adding an Effect

A handler returns effects. It does not perform them. That separation is what keeps `state.handle` pure.

```rust
let effects = state.handle(event).unwrap_or_default();
for effect in effects {
    // perform it
}
```

An effect is a variant on one enum, and the arm that performs it. Adding one is adding both.

## The variant

`MercuryEffect` is `mercury`'s, not freddie's. It says everything the program can be asked to do:

```rust
pub enum MercuryEffect {
    Foreground(App),
    Tap(Chord),
    Emit(KeyEvent),
    Place(Placement),
    Copy(Copied),
    Kill,
    ShowOverlay(&'static str),
    HideOverlay,
    ShowLayer(&'static str),
    Timer(TimerEffect<MercuryEvent>),
}
```

Carry the data the performer needs and nothing it can work out for itself. `Tap(Chord)` carries the key and the modifiers, because the emitter cannot guess them. `HideOverlay` carries nothing, because there is only one overlay.

Prefer a variant that says what to do over one that says how. `Place(Placement)` names a placement rather than a rectangle, so the arm that performs it is the only code that has to know how big the screen is.

## The arm

The effect loop is a match. Each arm reaches the crate that does the work:

```rust
match effect {
    MercuryEffect::Foreground(app) => foreground_app(app),
    MercuryEffect::Place(placement) => place_window(placement),
    MercuryEffect::Copy(what) => copy(what),
    MercuryEffect::HideOverlay => freddie_overlay::hide(),
    MercuryEffect::Kill => {
        info!("kill: exiting");
        return ControlFlow::Break(());
    }
    // ...
}
```

Two shapes show up there. Most arms hand off and return, and the work happens elsewhere: `foreground_app` and `place_window` each spawn, because an accessibility call can take tens of milliseconds and the loop has more effects to get through. `Kill` is the exception that answers the loop itself, breaking out of it.

An arm that fails logs and carries on. A key that could not be emitted is a `warn!`, not a panic, because the next effect in the vector may still be worth performing.

## Where it goes wrong

A handler that performs something instead of returning it takes `state.handle` out of the pure-function business, and everything downstream of that stops being true: the tests stop being a table, the log stops being a full record, and the same event stops producing the same result twice.

If a handler seems to need to do the thing itself, it usually needs a new variant.
