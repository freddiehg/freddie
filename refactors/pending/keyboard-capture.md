# input capture, from the ground up

Four layers, bottom-up, each independent of the ones above it:

1. a synchronous, generic pipeline that knows nothing about globals, I/O, or keys;
2. a global stack whose internal state is one such pipeline;
3. raw keyboard capture and emit, which knows nothing about pipelines;
4. the piece that registers (3) into (2) for a given event type.

Only layer 3 is keyboard-specific. Layers 1 and 2 are generic; layer 4 is generic except for a type that implements a trait.

## 1. Pipeline

A `Pipeline<T>` is a vec of steps that a value threads through, synchronously. No globals, no keys, no I/O, no async.

```rust
pub struct Pipeline<T>(/* Vec of steps */);
impl<T> Pipeline<T> {
    pub fn push(&self, step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;  // append; hand back the handle
    pub fn feed(&self, value: T);                                            // thread a value from the top
}

pub struct Step<T>(/* private */);
impl<T> Step<T> {
    pub fn inject(&self, value: T);   // push a value in at this step; it flows to the steps after it
}
// impl Drop for Step<T>: pops itself out of the pipeline.
```

`feed` runs a value through the steps in order; each step returns `Some(v)` to pass `v` on or `None` to drop it. `inject` is the second way in: whoever holds a `Step` can push a value at that step's position, and it flows downstream from there, not back through the steps before it. A `Step` removes itself on drop.

That is the whole primitive, synchronous and generic, testable with no I/O:

- Two steps, A->B then B->C: feeding A yields C at the end.
- A step that returns its input unchanged passes values it does not touch.
- Three steps, drop the middle: feeding threads the first straight to the last.
- `inject` at a step reaches only the steps after it.

## 2. The global stack

The pipeline is the internal state of a global stack, thread-local or process-static. You do not pass a `Pipeline` around; you push a step onto the one already there and get back a `Step` that pops on drop.

```rust
pub fn push<T>(step: impl Fn(T) -> Option<T> + 'static) -> Step<T>;   // onto the global Pipeline<T>
pub fn feed<T>(value: T);                                             // into the global Pipeline<T>
```

There is one global pipeline per `T`. Thread-local versus static is a choice (thread-local is simpler and fits `Rc`; static needs `Send`), but either way you create steps "in the global," which is the point of it being global.

## 3. Keyboard capture and emit

Separate and lower level: the raw keyboard I/O, with nothing about pipelines. You grab the keyboard, it hands you keys, and you can emit keys.

```rust
pub struct Grab(/* private */);
impl Grab {
    pub fn new(on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<Grab, CaptureError>;
    pub fn tap(&self, key: Key) -> Result<(), EmitError>;      // press then release
    pub fn press(&self, key: Key) -> Result<Press, EmitError>; // held; released when the last handle drops
}
// impl Drop for Grab: releases the keyboard.

pub struct Press(/* private */);
impl Clone for Press;   // another handle keeping the key down
// impl Drop for Press: the last handle releases the key.

pub struct CaptureError;                 // Display + Error
pub enum EmitError { Unmappable(Key), Post }
```

`on_key` is provided up front, so keys are never buffered; it runs on the capture thread, so keep it to a forward. `tap`/`press` emit; held keys are ref-counted per key (a per-key count and the event source live in the `Grab`, a `Press` holds an `Rc` reference plus its key, `Press::clone` adds a holder, `Press::drop` releases at zero). `Rc` makes `Press` `!Send`; a `send` Cargo feature swaps `Rc` for `Arc` (README TODO). There are no chords: cmd+r is `let _cmd = grab.press(Key::MetaLeft)?; grab.tap(Key::KeyR)?;`.

`Key` is one enum, a variant per key (`Key::KeyR`, `Key::Escape`, `Key::MetaLeft`, `Key::Raw(u16)`), so keys match, hash, and pass uniformly. `KeyEvent` is `{ key: Key, down: bool }`: which key, and down or up. Nothing else; keyboards are not velocity-sensitive.

Loop prevention is per process: an emit is stamped in the `USER_DATA` field with the process's tag, and the grab passes an event carrying its own tag straight through, so it never re-captures its own output while another process's grab (a different tag) still does.

On macOS the backend is one `CGEventTap`:

```rust
CGEventTap::with_enabled(
    Session, TailAppendEventTap, Default, [KeyDown, KeyUp, FlagsChanged],
    |_, kind, event| {
        if event.field(USER_DATA) == MY_TAG { return Keep; }   // our own output: let it out
        let key = from_code(event.field(KEYCODE));             // Key::Raw(code) if unnamed
        on_key(KeyEvent { key, down });
        Drop
    },
    || { ready.send(CFRunLoop::get_current()); CFRunLoop::run_current(); },
);
```

`with_enabled` does the run-loop wiring inside core-graphics, so we call no `unsafe`, and the crate stays inside the workspace `forbid(unsafe_code)`. `CFRunLoop` is `Send`, so `Grab::drop` stops the loop from any thread; a process exit or `kill -9` also frees the tap. Keycodes come from core-graphics' `KeyCode` constants; anything unnamed becomes `Key::Raw(code)`, so nothing is lost. Modifiers arrive as `FlagsChanged`, not `KeyDown`/`KeyUp`, so the tap listens for it and reads the side (`ShiftLeft` vs `ShiftRight`) from the keycode. An active tap needs Accessibility plus Input Monitoring.

## 4. The global capture

The thing we actually use ties layer 3 into layer 2. Nothing here is key-specific except a type that implements a trait knowing how to grab its raw capture and hand its events to the global pipeline:

```rust
pub trait Intercept: Sized {
    type Raw;   // the raw capture (Grab for keyboard)
    fn grab(on_event: impl Fn(Self) + Send + 'static) -> Result<Self::Raw, CaptureError>;
    fn emit(raw: &Self::Raw, value: &Self) -> Result<(), EmitError>;
}
// KeyEvent: Intercept { type Raw = Grab; grab = Grab::new; emit = tap/press }
```

Creating a global capture spins up the raw `Grab` and pushes a step into the global `Pipeline<KeyEvent>`. Raw keys `feed` the pipeline; a step transforms them; the pipeline's tail emits the result through the `Grab`. Dropping the capture pops the step and drops the `Grab`.

```rust
pub struct Capture<T: Intercept>(/* the Step, plus T::Raw */);
pub fn capture<T: Intercept>(step: impl Fn(T) -> Option<T> + 'static) -> Result<Capture<T>, CaptureError>;
```

Because layer 2 stacks and layer 1 appends, a second `capture` becomes the next step, so it sees the output of the one before it. Dropping any capture pops its step and heals the pipeline; you can add at the end but not the middle.

### Async emit

A step need not be synchronous end to end. mercury's step forwards the key into a tokio channel and returns `None` (drop it for now); the event loop decides, then `inject`s the result at that step, which flows to the steps after it. That is how the pipeline stays synchronous while the work is async, and why `inject` exists. The emit lands downstream of its own step, so no step sees its own output; position, not the tag, prevents the loop in-process (the tag is the per-process backstop). This is also why nothing is buffered in freddie_keyboard and it stays tokio-friendly without owning a runtime. A pull form (drain the keys yourself) is exposed too, as a footgun: it buffers unboundedly if you fall behind.

### Cross-process

Everything above is in-process: one global `Pipeline<KeyEvent>`, one `Grab`. Crossing processes is a separable layer with the same semantics: the per-process pipelines link through the global OS event-tap chain, which stands in for the vec's ordering, so one process's posted output is the next's tap input. The in-process pipeline knows nothing about it. Same-process is the requirement, cross-process the goal.

## What's left to build

The code today has `run(on_key)` + `emit(key)` + `emit_chord(mods, key)` on the core-graphics backend, with mercury driving it. To reach the layers above:

1. `Pipeline<T>` and `Step<T>` (push, feed, inject, drop-pops), pure and unit-tested.
2. The global `Pipeline<T>` stack (thread-local for now, `Rc`).
3. Rework `freddie_keyboard` to the raw `Grab` (grab/tap/press/Press, `Key::Raw`, `FlagsChanged`, per-process tag).
4. The `Intercept` trait, `KeyEvent`'s impl, and `capture<T>` wiring the raw grab into the global pipeline.
5. mercury: `capture` a step that forwards into a tokio channel (escape exits from it) and `inject`s the remap; one stage, so it re-emits everything.

## Known limits

- If macOS disables the tap on a timeout it does not auto re-enable (the cost of `with_enabled`, which keeps us unsafe-free). The callback is trivial, so it should not fire.
- F21-F24 have no macOS keycode, so emitting them returns `Unmappable` until we learn what real hardware sends. `Key::Raw` is the escape hatch, and its `u16` is the one non-portable value.
