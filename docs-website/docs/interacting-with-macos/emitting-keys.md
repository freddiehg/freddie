---
title: Emitting Keys
sidebar_position: 3
---

# Emitting Keys

## The emitter

`intercept` returns both halves at once, and they share a tag because one call made them:

```rust
let (interceptor, emitter) =
    freddie_keyboard::intercept(callback)?;
```

The `Emitter` posts keys. It has two methods, both taking the flags the event is to carry:

```rust
impl Emitter {
    pub fn emit(
        &self,
        key: Key,
        press: PressType,
        flags: ModifierFlags,
    ) -> Result<(), EmitError>;

    pub fn tap(
        &self,
        key: Key,
        flags: ModifierFlags,
    ) -> Result<(), EmitError>;
}
```

`tap` is a press and then a release, both carrying the same flags. Each posted event is built from a `CGEventSource` created for it and dropped with it: posting through a source mutates it, so a single arrow key would leave `NumericPad` in a long-lived one and every key after it would be born carrying that bit, which is enough to stop `cmd`-`space` being the Spotlight hotkey for the rest of the run.

The model never calls either. It returns effects, and the effect loop performs them:

```rust
pub enum MercuryEffect {
    Tap(Chord),
    Emit(KeyEvent),
    // ...
}

pub struct Chord {
    pub key: Key,
    pub flags: ModifierFlags,
}
```

A chord is one key event with its modifiers baked in as flags. `cmd`-`r` is `Chord { key: KeyR, flags: COMMAND }`, not a synthetic `cmd` down and up around an `r`: that extra up would strand the modifier the user is really holding, because the app counts it and thinks the key was released.

`Emit` is the escape hatch for the one case that genuinely is a lone half of a keypress, which is passing a key through. The model sees a down and an up as separate events and re-emits each. Building a chord out of two `Emit`s is a bug waiting to happen.

The effect loop is the single consumer of the effect channel, and it runs on the worker thread that owns the state, so effects reach the OS in the order dispatch produced them. A modifier goes out before the key carrying its flag.

## Passthrough

Every key is swallowed. The tap callback always returns `None`, so nothing reaches the app natively, and a key that passes through does so as an emitted effect on the same ordered pipeline as every remap.

That is not an accident of the design. Passing unbound keys natively while swallowing the bound ones reorders them: a natively-passed key reaches the app immediately, while a swallowed one is still going through the channel and back out. Type `a b a` with `b` remapped and the app can see `a a B`.

The root binds the catch-all last, so a key any layer bound never reaches it:

```rust
#[bind(
    // ...
    AnyKey => maybe_pass_through,
)]
pub struct Mercury { /* ... */ }
```

`maybe_pass_through` records a modifier in `held` first, then splits on the layer. Outside a passthrough layer the key is swallowed and that is the whole story. Inside one it goes to the `jk` run, which either takes it, hands back what it had swallowed for a key that broke the run, or completes and leaves for home. A key the run does not want is emitted with exactly the flags it arrived with, so a modifier baked onto the event rather than delivered as its own key, an injected `cmd`-`v` or anything carrying `fn`, rides along:

```rust
out.push(emit(ev.key, ev.press, ev.flags));
```

`Layer::is_passthrough` is the one test, and typing is the only layer it holds for.

## Why an emitted key does not come back

Posting an event puts it back at the top of the tap chain, so `mercury`'s own output arrives at `mercury`'s own tap. The emitter stamps a tag on everything it posts, and the callback checks that before anything else:

```rust
if tag.marks(event) {
    return CallbackResult::Keep; // our own emit
}
```

The tag is an `i64` written into `EventField::EVENT_SOURCE_USER_DATA`, and it is random per process rather than a well-known constant. Two freddie processes sharing a constant would each wave the other's output through as if it were their own.

The tag stops a process from eating its own output. It does not stop two remappers with inverse maps from feeding each other, since neither one's tag matches the other's. A returned event never re-enters the top of the chain, so the remap-by-return path is loop-free without a tag at all; only the emitter needs one.
