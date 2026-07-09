# keyboard capture and emit

Grab the keyboard on macOS. The callback decides each key and returns what to send on, and the OS tap chain runs that as a pipeline. v1 is one process with one grab.

## The tap chain is the pipeline

A `CGEventTap` is installed at a location (`kCGSessionEventTap`) with a placement (`HeadInsertEventTap` or `TailAppendEventTap`). The system keeps an ordered list of the active taps there, and every event runs through them in order: each callback returns the event to pass it, a different event to replace it, or NULL to drop it, and the result feeds the next tap, then the app. That is the pipeline: a tap is a step, pass/replace/drop are its outcomes, and the app is the outlet. The OS keeps the list and runs it across processes, so we do not build a pipeline, we are one entry in one.

It is correct and loop-free as long as a tap decides by returning the event, because the event only ever moves forward and is never re-posted. The loop comes from `CGEventPost`: a posted event re-enters at the top of the chain rather than continuing down it. That is what forces the tag, and it is what an async decision forces, since a callback can't hold the event while it waits. So the primary path is to decide in the callback and return the remapped event.

## API

```rust
pub struct Grab(/* private */);
impl Grab {
    // Decides each key and returns what continues: Some(same) passes, Some(other) remaps, None drops.
    pub fn new(on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static) -> Result<Grab, CaptureError>;
    // Synthesize a key not tied to the current event. Re-posts, so it carries the tag.
    pub fn emit(&self, key: Key, press: PressType) -> Result<(), EmitError>;
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

`on_key` runs on the capture thread and returns the transform, so keep it fast; freddie's remap is CPU-only, so it fits. `emit`, `tap`, and `press` are for keys the callback did not produce by returning, a macro or a held modifier. They `CGEventPost`, so they carry the tag and hit the cross-process caveat below. `press` holds a key and releases it when the last `Press` drops, ref-counted per key; `Press` is `Rc`-backed and `!Send`, and a `send` feature swaps in `Arc`.

`Key` is one enum, a variant per key, `Key::Raw(u16)` included. `KeyEvent` is `{ key: Key, press: PressType }`, and `PressType` is `Down` or `Up`, a named enum rather than a bare bool.

## The tag, and the cross-process limit

A re-posted event comes back around to our own tap, so each grab stamps its emits with a tag in `USER_DATA` and passes an event carrying its own tag straight through. The return-the-event path needs no tag; only `emit`/`tap`/`press` do.

The tag stops a grab looping on itself, not on another process. Two remappers with inverse maps loop: A grabs and re-posts B, B grabs and re-posts A, and B's post of A carries B's tag, not A's. Returning the event instead of posting avoids this, because a returned event never re-enters the top. Karabiner sidesteps the whole thing by not using taps for the remap: it seizes the physical keyboard at the IOKit HID level and emits through a virtual HID keyboard from a DriverKit driver, so its output is never its input. That is the correct cross-process design and a signed-driver project. v1 is single-process, where returning the event is correct and the tag covers the rest.

## The macOS backend

```rust
CGEventTap::with_enabled(
    Session, TailAppendEventTap, Default, [KeyDown, KeyUp, FlagsChanged],
    |_, kind, event| {
        if event.field(USER_DATA) == MY_TAG { return Keep; }   // our own re-post
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

`with_enabled` runs the run-loop wiring inside core-graphics, so nothing here is `unsafe` and the crate stays under the workspace `forbid(unsafe_code)`. `CFRunLoop` is `Send`, so `Grab::drop` stops it from any thread, and killing the process frees the tap regardless. Key codes come from core-graphics' `KeyCode` table; an unnamed code becomes `Key::Raw(code)`. Modifiers arrive as `FlagsChanged` rather than `KeyDown`/`KeyUp`, and the keycode gives the side (`ShiftLeft` vs `ShiftRight`). The tap needs Accessibility and Input Monitoring.

## v1 scope

One process, one `Grab`, deciding each key in the callback and returning the result. `emit`/`tap`/`press` for synthesized keys. Cross-process correctness needs the virtual-HID route and is out of scope.

## Known limits

- If macOS disables the tap after a slow callback it will not turn it back on by itself. `with_enabled` keeps us out of `unsafe` but hides the tap handle, and the callback is fast, so it should not happen.
- F21 through F24 have no macOS keycode, so emitting them returns `Unmappable` until we find out what real hardware sends. `Key::Raw` is the way around it, and its `u16` is the one value that is not portable.
