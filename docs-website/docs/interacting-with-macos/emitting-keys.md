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

`tap` is a press and then a release, both carrying the same flags.

Every event is built from one long-lived `CGEventSource`, created once per thread that builds events: the emitter holds one and the tap thread holds one for remaps. A source per event would map about 16KB of window server memory per keystroke that `CFRelease` never unmaps, which is [a gigabyte in five hours](../freddie-internals/owning-os-resources.md). Reusing one is safe because the emitted flags are `to_cg(flags) | intrinsic_flags(code)` and never the bits the event was born with. Posting through a source mutates its flag state, so an arrow leaves `NumericPad` in it, and a `cmd`-`space` built afterwards that inherited that bit would stop matching Spotlight's hotkey for the rest of the run.

Posting also runs inside an `objc2::rc::autoreleasepool`, because `CGEventPost` autoreleases two `CFData`s per call and the effect loop's thread has no run loop to drain them. See [Autorelease Pools](../freddie-internals/autorelease-pools.md).

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

## Why an emitted key does not re-enter the model

Without a filter, every emit would re-enter the same tap, become another event, dispatch, emit again, and loop. Posting puts the event back at the head of the session tap chain, so the interceptor would otherwise treat its own output as a new physical key.

`intercept` builds one random tag for both halves. The emitter stamps it onto every post (`EventField::EVENT_SOURCE_USER_DATA`). The tap checks it before anything else and, on a match, returns `CallbackResult::Keep`: the event continues to the rest of the system (and eventually the front app), but the app callback never runs and the model never sees it.

```rust
if tag.marks(event) {
    return CallbackResult::Keep; // our own emit — pass through, do not re-handle
}
```

Physical input has no tag, so it is still sent to the model as before. The tag is per-process random rather than a well-known constant, so two freddie processes do not each wave the other's output through as if it were their own.

The filter only breaks the self-loop. It does not stop two remappers with inverse maps from feeding each other, since neither tag matches the other. A returned event (remap-by-return from the callback) never re-enters the top of the chain, so that path is loop-free without a tag; only the emitter needs one.
