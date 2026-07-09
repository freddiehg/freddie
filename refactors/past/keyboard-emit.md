# emitting keyboard events

Performing the outward key effects (`Type`, `Command`, and the re-emit every passed key needs under swallow-all) by synthesizing key events. Same three layers as capture: `rdev::simulate` does the OS work, `freddie_keyboard::emit` is the shared wrapper, the consumer's effect loop calls it. This is the detail.

## Crate organization

- `rdev::simulate` posts a synthetic key event to the OS (`CGEventPost` on macOS). The existing library.
- `freddie_keyboard::emit` wraps it: one call presses and releases a key, and marks the event so a running `run` interceptor lets our own output through instead of re-handling it. Shared, because every consumer that hijacks needs both the emit and the self-feedback guard.
- The consumer's effect loop decides what to emit and when (a `Type`, a `Command`, a passthrough re-emit). That mapping is app policy, not the crate's.

## The freddie_keyboard::emit API

What exists:

```rust
pub fn emit(key: Key) -> Result<(), Error> {
    SYNTHETIC.fetch_add(2, Ordering::SeqCst); // one for the press, one for the release
    rdev::simulate(&rdev::EventType::KeyPress(key)).map_err(Error::Simulate)?;
    rdev::simulate(&rdev::EventType::KeyRelease(key)).map_err(Error::Simulate)?;
    Ok(())
}
```

On macOS `simulate` is `CGEventPost` and needs Accessibility. The effect loop is a single consumer, so emits happen one at a time and in order, which is what typing needs and what keeps passthrough in order (passthrough.md).

## v1 does not emit

mercury v1 prints instead of emitting. The effect loop today:

```rust
MercuryEffect::Type(s) => println!("printed {s}"),
MercuryEffect::Command(k) => println!("send cmd+{k}"),
```

So `emit` is exercised by the real remapper (figaro, or mercury past v1), not by v1. Swapping those prints for `freddie_keyboard::emit(key)`, and a chord emit for `Command`, is the whole move from v1's echo to a real remapper.

## Self-feedback

Posted events come back to our own `run` callback, so without a guard we would capture and re-dispatch our own output forever. rdev does not tag synthetic events, so `freddie_keyboard` keeps a `SYNTHETIC` counter: `emit` adds 2 (press + release), and `run`'s callback decrements and passes an event through while it is positive:

```rust
if SYNTHETIC.load(Ordering::SeqCst) > 0 {
    SYNTHETIC.fetch_sub(1, Ordering::SeqCst);
    return Some(event); // our own key: let it through, do not re-dispatch
}
```

This is racy, and swallow-all makes it worse, not better. Under swallow-all every passed key is re-emitted (passthrough.md), so `SYNTHETIC` is under near-constant load; a real physical key landing between an `emit`'s `fetch_add` and the callback consuming the two synthetic events can be mistaken for ours, which passes it to the app un-dispatched. The counter is fine for v1 (which never emits) and a stopgap for a light remapper, but a real one wants an exact tag:

- A real tag. Raw `core-graphics` can set `kCGEventSourceUserData` on the posted event, and the callback reads it back: exact, not racy, load-independent. rdev does not expose event fields, so this means dropping to `core-graphics` for the emit and the callback read only, and that module would opt out of the workspace `forbid(unsafe_code)`. This is the "unless we need to write it" case, and swallow-all makes it likely required, not optional.

## Chords and named keys

`emit` does one key. The rest is not in the crate yet:

- A chord like cmd+r is a sequence: press `MetaLeft`, press `KeyR`, release `KeyR`, release `MetaLeft`. rdev has no flags-on-a-key API, so the modifier is its own events around the key and order matters (modifier down before the key, up after). Either the crate grows `emit_chord(mods, key)` or the effect loop sequences raw `emit`/`simulate` calls:

```rust
// not in freddie_keyboard yet:
pub fn emit_chord(mods: &[Key], key: Key) -> Result<(), Error>;
// cmd+r == emit_chord(&[Key::MetaLeft], Key::KeyR)
```

- Named keys (escape, arrows, function keys) are just their `rdev::Key` variants; nothing special.
- Layout-independent text: rdev is key-code based, so `emit` produces whatever the key maps to under the current layout. Arbitrary Unicode without a key code (macOS `CGEventKeyboardSetUnicodeString`) is not exposed by rdev; that is another `core-graphics` path if we ever need it. A remapper rarely does (we re-emit real keys), so defer.

## Re-emit (passthrough and remap)

Under swallow-all, everything that should reach the app is re-emitted through `emit`, in order (passthrough.md):

- Passthrough: swallow the original, re-emit the same key. The effect carries the original key.
- Remap: swallow the original, emit the replacement (a key, a chord, a `Command`).

There is no native-pass path; that reordered keys and is gone (keyboard-capture.md). So every non-consumed key is one `emit`, which is why emit volume is high and the self-feedback tag matters.

## Modifiers and hold

Simple emits are press-then-release immediately. Anything that holds a modifier across several keys, or holds a key, has to emit the press, do the keys, then the release, tracking state. On the input side this is the same idea as modeling held modifiers as layers (modifier-keys.md).

## Open questions

- Counter guard vs a real `core-graphics` tag: swallow-all keeps the counter under constant load, so is the tag (and its `unsafe` FFI) now required rather than optional?
- Do we ever need Unicode/text emit (raw `core-graphics`), or is key-code re-emit always enough?
- `emit_chord` in the crate versus sequencing raw `emit` in the effect loop, and whether chords need small sleeps for apps to register the modifier.
- Does re-emitting a swallowed key preserve everything the app needs (exact modifiers), or do subtleties leak?
