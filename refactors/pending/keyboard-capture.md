# input capture, from the ground up

Four pieces. They don't form a stack where each sits on the last.

- The pipeline (1) is generic and synchronous, with no globals, keys, or I/O in it.
- The global stack (2) holds a pipeline, so it needs (1).
- Raw keyboard capture and emit (3) has no idea pipelines exist. It needs nothing.
- The global capture (4) drops a raw keyboard into the global pipeline, so it needs (2) and (3).

So (1) and (2) are one half, (3) is a separate half that knows nothing about the first, and (4) is the only place they touch. Keys live in (3) and in one trait impl (4) uses; everything else is generic.

## 1. Pipeline

`Pipeline<T>` is a vec of steps. You feed a value in at the top and it runs through the steps in order. Synchronous, generic, no I/O.

```rust
pub struct Pipeline<T>(/* Vec of steps */);
impl<T> Pipeline<T> {
    pub fn push(&self, step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;  // append, hand back the handle
    pub fn feed(&self, value: T);                                            // start a value at the top
}

pub struct Step<T>(/* private */);
impl<T> Step<T> {
    pub fn inject(&self, value: T);   // drop a value in right after this step
}
// impl Drop for Step<T>: takes itself back out.
```

A step is `Fn(T) -> Option<T>`. Return `Some(v)` and `v` goes on to the next step; return `None` and the value stops there. `feed` starts a value at the top. `inject` is the other door in: hold a `Step` and you can put a value into the pipeline right below it, hitting the steps after it but not the ones before. Drop a `Step` and it comes out.

None of this needs a keyboard to test:

- Two steps, A to B then B to C: feed A, get C.
- A step that hands its input straight back leaves values it doesn't care about alone.
- Three steps, drop the middle: the first now feeds the last.
- `inject` at a step reaches the steps below it and no others.

## 2. The global stack

You don't pass the pipeline around. It sits in a global, one per `T`, and you push steps onto that one. Push gives you a `Step`; drop it and the step comes off.

```rust
pub fn push<T>(step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;   // onto the global Pipeline<T>
pub fn feed<T>(value: T);                                             // into the global Pipeline<T>
```

Thread-local or process-static is a real choice. Thread-local is simpler and lets steps hold `Rc`; static forces `Send`. Either way you push onto the global instead of threading a `Pipeline` through your code.

## 3. Keyboard capture and emit

The raw keyboard, with nothing about pipelines in it. Grab the keyboard, get keys through a callback, emit keys back.

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

You hand over `on_key` when you grab, so keys never pile up in a buffer waiting for someone to read them. It runs on the capture thread, so it should pass the key along and get out of the way. `tap` and `press` emit. A held key is ref-counted: the count and the event source sit in the `Grab`, a `Press` is a reference plus the key it holds, cloning it adds a holder, and dropping the last one sends the release. `Press` holds an `Rc` today, so it is `!Send`; a `send` feature swaps in `Arc` (README TODO). There are no chords. cmd+r is `let _cmd = grab.press(Key::MetaLeft)?; grab.tap(Key::KeyR)?;`.

`Key` is one enum with a variant per key, `Key::Raw(u16)` among them. `KeyEvent` is the key and whether it went down or up, and nothing else. Keyboards don't report velocity.

Every emit gets stamped with the process's tag in the event's `USER_DATA` field. The grab lets an event carrying its own tag pass straight through, so it never re-captures what it just emitted; another process's grab has a different tag and does capture it.

On macOS it is one `CGEventTap`:

```rust
CGEventTap::with_enabled(
    Session, TailAppendEventTap, Default, [KeyDown, KeyUp, FlagsChanged],
    |_, kind, event| {
        if event.field(USER_DATA) == MY_TAG { return Keep; }   // our own output, let it out
        let key = from_code(event.field(KEYCODE));             // Key::Raw(code) if unnamed
        on_key(KeyEvent { key, down });
        Drop
    },
    || { ready.send(CFRunLoop::get_current()); CFRunLoop::run_current(); },
);
```

`with_enabled` runs the run-loop wiring inside core-graphics, so none of this is `unsafe` and the crate stays under the workspace `forbid(unsafe_code)`. `CFRunLoop` is `Send`, so `Grab::drop` stops the loop from whatever thread drops it, and killing the process frees the tap anyway. Key codes come from core-graphics' `KeyCode` table, and a code with no name becomes `Key::Raw(code)`, so nothing gets dropped for being unnamed. Modifiers arrive as `FlagsChanged`, not `KeyDown`/`KeyUp`, and the keycode says which side. The tap needs Accessibility and Input Monitoring.

## 4. The global capture

This is what you actually reach for. It wires a raw `Grab` into the global pipeline. The one key-specific thing is a type that implements a trait saying how to grab the device and how to emit through it.

```rust
pub trait Intercept: Sized {
    type Raw;   // Grab, for keyboard
    fn grab(on_event: impl Fn(Self) + Send + 'static) -> Result<Self::Raw, CaptureError>;
    fn emit(raw: &Self::Raw, value: &Self) -> Result<(), EmitError>;
}
// KeyEvent: Intercept { type Raw = Grab; grab = Grab::new; emit = tap/press }
```

Creating a capture starts a `Grab` and pushes a step into the global `Pipeline<KeyEvent>`. Real keys feed the pipeline, the step does its work, and the pipeline's last step emits through the `Grab`. Drop the capture and its step comes off and the `Grab` goes with it.

```rust
pub struct Capture<T: Intercept>(/* the Step, plus T::Raw */);
pub fn capture<T: Intercept>(step: impl Fn(T) -> Option<T> + 'static) -> Result<Capture<T>, CaptureError>;
```

A second capture lands after the first, so it works on what the first already changed. Drop one out of the middle and the pipeline closes up. You can add at the end but not wedge into the middle.

### Async emit

A step doesn't have to finish synchronously. mercury's step drops the key into a tokio channel and returns `None`. Later its event loop decides what to do and `inject`s the result at that step, and it flows to the steps below. The pipeline stays synchronous while the real work happens off it, and `inject` is the bridge. The injected key lands below its own step, so no step ever sees what it emitted, and that alone stops the loop in one process; the per-process tag is a backstop. Nothing buffers inside freddie_keyboard and it never touches a runtime, so tokio is the consumer's call. There is also a pull form that hands you the keys to drain yourself, which is a footgun: fall behind and it grows without bound, so it is opt-in.

### Cross-process

Everything above is one process, one global pipeline, one `Grab`. Spanning processes is a separate layer that behaves the same way. Each process runs its own pipeline, and the OS event-tap chain links them in place of a single vec, so one process's output is the next one's input. The in-process pipeline has no idea this is going on. Same-process is required, cross-process is the goal.

## What's left to build

Today the crate has `run(on_key)` + `emit(key)` + `emit_chord(mods, key)` on the core-graphics backend, with mercury driving it. To get to the four pieces:

1. `Pipeline<T>` and `Step<T>`: push, feed, inject, drop-pops. Pure, unit-tested.
2. The global `Pipeline<T>` behind a thread-local (`Rc` for now).
3. Rework `freddie_keyboard` into the raw `Grab`: grab, tap, press, `Press`, `Key::Raw`, `FlagsChanged`, the per-process tag.
4. The `Intercept` trait, `KeyEvent`'s impl, and `capture<T>` wiring the grab into the global pipeline.
5. mercury: `capture` a step that forwards into a tokio channel, exits on escape, and `inject`s the remap. One stage, so it re-emits everything.

## Known limits

- If macOS disables the tap after a slow callback it won't turn it back on by itself. `with_enabled` keeps us out of `unsafe` but hides the tap handle, and the callback is trivial, so it shouldn't happen.
- F21 through F24 have no macOS keycode, so emitting them returns `Unmappable` until we find out what real hardware sends. `Key::Raw` is the way around it, and its `u16` is the one value that isn't portable.
