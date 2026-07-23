# keyboard capture and emit

Intercept the keyboard on macOS. The callback decides each key and returns what to send on, and the OS tap chain runs that as a pipeline. Two structs: an `Interceptor` that captures, and an `Emitter` that synthesizes keys. v1 is one process.

## The tap chain is the pipeline

A `CGEventTap` is installed at a location (`kCGSessionEventTap`) with a placement (`HeadInsertEventTap` or `TailAppendEventTap`). The system keeps an ordered list of the active taps there, and every event runs through them in order: each callback returns the event to pass it, a different event to replace it, or NULL to drop it, and the result feeds the next tap, then the app. That is the pipeline: a tap is a step, pass/replace/drop are its outcomes, and the app is the outlet. The OS keeps the list and runs it across processes, so we do not build a pipeline, we are one entry in one.

It is correct and loop-free as long as a tap decides by returning the event, because the event only ever moves forward and is never re-posted. The loop comes from `CGEventPost`: a posted event re-enters at the top of the chain rather than continuing down it. That is what forces the tag, and it is what an async decision forces, since a callback can't hold the event while it waits. So the primary path is to decide in the callback and return the remapped event.

## API

Capture and emit are separate structs, one handles input and the other produces output, but a single call makes both so they share a tag.

```rust
// Grab the keyboard. Returns both halves, sharing a tag so the interceptor passes the emitter's output.
// on_key decides each key: Some(same) passes, Some(other) remaps, None drops.
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError>;

pub struct Interceptor(/* private */);
// impl Drop for Interceptor: releases the keyboard.

pub struct Emitter(/* private */);
impl Emitter {
    // Synthesize a key not tied to an intercepted event. Re-posts, so it carries the tag.
    // The event states its own modifier flags rather than trusting the source's state.
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError>;
    pub fn tap(&self, key: Key, flags: ModifierFlags) -> Result<(), EmitError>; // down then up, both carrying flags
}

pub struct CaptureError;                 // Display + Error
pub enum EmitError { Unmappable(Key), Post }
```

```rust
let (interceptor, emitter) = intercept(on_key)?;
```

`on_key` runs on the capture thread and returns the transform, so keep it fast; freddie's remap is CPU-only, so it fits. The `Emitter` is for keys the callback did not produce by returning (a synthesized chord, or a modifier emitted down and up on its own), and it shares the interceptor's tag from the same `intercept` call. `emit`/`tap` `CGEventPost`, so they carry the tag and hit the cross-process caveat below, and both state the modifier flags on the event they post rather than trusting the source's state.

`Key` is one enum, a variant per key, `Key::Raw(u16)` included. `KeyEvent` is `{ key: Key, press: PressType, flags: ModifierFlags }`, `PressType` is `Down` or `Up`, a named enum rather than a bare bool, and `ModifierFlags` is a portable bitset the backend maps to native flags on emit.

## The tag, and the cross-process limit

A re-posted event comes back around to our own tap, so the emitter stamps a tag in `USER_DATA` and the interceptor passes an event carrying that tag straight through. They share the one value because a single `intercept` call makes both. The return-the-event path needs no tag; only the emitter does, so a return-only setup (the simple mercury) never touches it.

Make the tag unique per process rather than a well-known constant. Two freddie processes sharing a constant would each skip the other's output as if it were their own, breaking the cross-process chain. A random tag at startup avoids that for free. Even so, the tag stops a process looping on itself, not on another process: two remappers with inverse maps loop, A re-posts B and B re-posts A, and neither tag matches the other's. Returning the event instead of posting avoids that, since a returned event never re-enters the top. Karabiner sidesteps the whole class by not using taps for the remap (virtual-hid.md). v1 is single-process, where returning the event is correct and the tag covers the emitter.

## The macOS backend

```rust
CGEventTap::with_enabled(
    Session, TailAppendEventTap, Default, [KeyDown, KeyUp, FlagsChanged],
    |_, kind, event| {
        if event.field(USER_DATA) == my_tag { return Keep; }   // our own re-post
        let key = from_code(event.field(KEYCODE));             // Key::Raw(code) if unnamed
        let press = if kind == KeyDown { PressType::Down } else { PressType::Up };
        match on_key(KeyEvent { key, press }) {
            Some(out) if out.key == key && out.press == press => Keep,
            Some(out) => Replace(encode(out)),
            None => Drop,
        }
    },
    || { ready.send(CFRunLoop::get_current()); CFRunLoop::run_current(); },
);
```

`with_enabled` runs the run-loop wiring inside core-graphics, so nothing here is `unsafe` and the crate stays under the workspace `forbid(unsafe_code)`. `CFRunLoop` is `Send`, so `Interceptor::drop` stops it from any thread, and killing the process frees the tap regardless. Key codes come from core-graphics' `KeyCode` table; an unnamed code becomes `Key::Raw(code)`. Modifiers arrive as `FlagsChanged` rather than `KeyDown`/`KeyUp`, and the keycode gives the side (`ShiftLeft` vs `ShiftRight`). The tap needs Accessibility and Input Monitoring.

## v1 scope

One process: an `Interceptor` deciding each key in the callback and returning the result, and an `Emitter` for synthesized keys. Cross-process correctness needs the virtual-HID route and is out of scope.

## Known limits

- If macOS disables the tap after a slow callback it will not turn it back on by itself. `with_enabled` keeps us out of `unsafe` but hides the tap handle, and the callback is fast, so it should not happen.
- F21 through F24 have no macOS keycode, so emitting them returns `Unmappable` until we find out what real hardware sends. `Key::Raw` is the way around it, and its `u16` is the one value that is not portable.
