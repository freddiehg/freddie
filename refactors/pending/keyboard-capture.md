# input capture, from the ground up

Four pieces, not a stack. The pipeline (1) is generic and synchronous. The global stack (2) holds a pipeline, so it depends on 1. Raw keyboard capture and emit (3) is unrelated to either. The global capture (4) puts a raw keyboard into the global pipeline, so it depends on 2 and 3. Two independent halves, 1-2 and 3, joined only at 4. Keys appear only in 3 and in the trait impl 4 uses.

## 1. Pipeline

`Pipeline<T>` is a vec of steps. `feed` a value at the top and it runs through the steps in order. Synchronous and generic.

```rust
pub struct Pipeline<T>(/* Vec of steps */);
impl<T> Pipeline<T> {
    pub fn push(&self, step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;
    pub fn feed(&self, value: T);
}

pub struct Step<T>(/* private */);
impl<T> Step<T> {
    pub fn inject(&self, value: T);   // enters the pipeline right after this step
}
// impl Drop for Step<T>: removes itself.
```

A step is `Fn(T) -> Option<T>`: `Some(v)` passes `v` to the next step, `None` stops it. `feed` enters at the top. `inject` enters below a given step and reaches the steps after it, not the ones before. A `Step` removes itself on drop.

Testable with no keyboard:

- A to B then B to C: feed A, get C.
- A step that returns its input unchanged leaves keys it does not touch alone.
- Drop the middle of three: the first feeds the last.
- `inject` reaches only downstream.

## 2. The global stack

The pipeline lives in a global, one per `T`. You push steps onto it instead of passing it around.

```rust
pub fn push<T>(step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;
pub fn feed<T>(value: T);
```

Thread-local at first: simpler, and it lets steps hold `Rc`. A process-static one (which requires `Send`) can follow later, with the `send` feature.

## 3. Keyboard capture and emit

The raw keyboard, no pipeline involved. Grab it, get keys through a callback, emit keys.

```rust
pub struct Grab(/* private */);
impl Grab {
    pub fn new(on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<Grab, CaptureError>;
    pub fn tap(&self, key: Key) -> Result<(), EmitError>;      // press then release
    pub fn press(&self, key: Key) -> Result<Press, EmitError>; // held until the last handle drops
}
// impl Drop for Grab: releases the keyboard.

pub struct Press(/* private */);
impl Clone for Press;   // another handle holding the key down
// impl Drop for Press: the last handle releases the key.

pub struct CaptureError;                 // Display + Error
pub enum EmitError { Unmappable(Key), Post }
```

`on_key` is given up front, so keys are not buffered. It runs on the capture thread, so keep it short. `tap` and `press` emit. A held key is ref-counted: the count and the event source live in the `Grab`, a `Press` references them plus its key, cloning adds a holder, the last drop releases. `Press` is `Rc`-backed and `!Send`; a `send` feature swaps in `Arc` (README TODO). No chords: cmd+r is `let _cmd = grab.press(Key::MetaLeft)?; grab.tap(Key::KeyR)?;`.

`Key` is one enum, a variant per key, `Key::Raw(u16)` included. `KeyEvent` is the key plus down or up, nothing more. Keyboards do not report velocity.

Emits carry the process's tag in the event's `USER_DATA` field. A grab passes its own tag through untouched and so ignores its own output; a different process's grab has a different tag and captures it.

macOS is one `CGEventTap`:

```rust
CGEventTap::with_enabled(
    Session, TailAppendEventTap, Default, [KeyDown, KeyUp, FlagsChanged],
    |_, kind, event| {
        if event.field(USER_DATA) == MY_TAG { return Keep; }   // our own output
        let key = from_code(event.field(KEYCODE));             // Key::Raw(code) if unnamed
        on_key(KeyEvent { key, down });
        Drop
    },
    || { ready.send(CFRunLoop::get_current()); CFRunLoop::run_current(); },
);
```

`with_enabled` handles the run loop inside core-graphics, so nothing here is `unsafe` and the crate stays under the workspace `forbid(unsafe_code)`. `CFRunLoop` is `Send`, so `Grab::drop` stops it from any thread, and killing the process frees the tap regardless. Key codes come from core-graphics' `KeyCode` table; an unnamed code becomes `Key::Raw(code)`. Modifiers arrive as `FlagsChanged` rather than `KeyDown`/`KeyUp`, and the keycode gives the side. The tap needs Accessibility and Input Monitoring.

## 4. The global capture

What you actually use. It puts a raw `Grab` into the global pipeline. The only key-specific part is a type implementing a trait for how to grab and how to emit.

```rust
pub trait Intercept: Sized {
    type Raw;   // Grab, for keyboard
    fn grab(on_event: impl Fn(Self) + Send + 'static) -> Result<Self::Raw, CaptureError>;
    fn emit(raw: &Self::Raw, value: &Self) -> Result<(), EmitError>;
}
// KeyEvent: Intercept { type Raw = Grab; grab = Grab::new; emit = tap/press }
```

A capture starts a `Grab` and pushes a step into the global `Pipeline<KeyEvent>`. Keys feed the pipeline, the step transforms them, the last step emits through the `Grab`. Dropping the capture removes the step and the `Grab`.

```rust
pub struct Capture<T: Intercept>(/* the Step, plus T::Raw */);
pub fn capture<T: Intercept>(step: impl Fn(T) -> Option<T> + 'static) -> Result<Capture<T>, CaptureError>;
```

A new capture lands after the existing ones, so it sees their output. Dropping one from the middle closes the gap. You append at the end, never insert in the middle.

### Async emit

A step can defer. mercury's step pushes the key into a tokio channel and returns `None`; its event loop later `inject`s the result at that step, downstream. The pipeline stays synchronous while the work runs async off to the side. The injected key lands below its own step, so no step sees its own output, which prevents the loop in-process; the per-process tag is a backstop. Nothing buffers in freddie_keyboard and it owns no runtime, so tokio is the consumer's choice. A pull form that hands you the keys to drain is also available, and it is a footgun: fall behind and it grows unbounded.

### Cross-process

All of the above is one process. Spanning processes is a separate layer with the same behavior: each process runs its own pipeline, and the OS event-tap chain orders them in place of a vec, so one process's output feeds the next. Same-process is required, cross-process is the goal.

## What's left to build

Today the crate has `run(on_key)` + `emit(key)` + `emit_chord(mods, key)` on the core-graphics backend, with mercury driving it. To get to the four pieces:

1. `Pipeline<T>` and `Step<T>`: push, feed, inject, drop-pops. Pure, unit-tested.
2. The global `Pipeline<T>` behind a thread-local (`Rc` for now).
3. Rework `freddie_keyboard` into the raw `Grab`: grab, tap, press, `Press`, `Key::Raw`, `FlagsChanged`, the per-process tag.
4. The `Intercept` trait, `KeyEvent`'s impl, and `capture<T>` wiring the grab into the global pipeline.
5. mercury: `capture` a step that forwards into a tokio channel, exits on escape, and `inject`s the remap. One stage, so it re-emits everything.

## Known limits

- If macOS disables the tap after a slow callback it will not turn it back on by itself. `with_enabled` keeps us out of `unsafe` but hides the tap handle, and the callback is trivial, so it should not happen.
- F21 through F24 have no macOS keycode, so emitting them returns `Unmappable` until we find out what real hardware sends. `Key::Raw` is the way around it, and its `u16` is the one value that is not portable.
