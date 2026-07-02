# emitting keyboard events

Performing the outward key effects (`Type`, `Command`, and the re-emit that passthrough and remaps need) by synthesizing key events. `freddie_keyboard::emit` over `rdev::simulate`. This is the detail.

## The basic emit

`emit(key)` posts a press then a release:

```rust
rdev::simulate(&EventType::KeyPress(key))?;
rdev::simulate(&EventType::KeyRelease(key))?;
```

On macOS this is `CGEventPost`, and it needs Accessibility. The effect loop is a single consumer, so emits happen one at a time and in order, which is what typing needs.

## Self-feedback

Posted events are delivered to our own `grab`/`listen` callback, so without a guard we would capture and re-dispatch our own output forever. rdev does not tag synthetic events, so `freddie_keyboard` keeps a `SYNTHETIC` counter: `emit` bumps it, the callback decrements and passes an event through while it is positive.

This is racy: a real key landing between the bump and the callback seeing the synthetic one can be mis-attributed. Two ways out:

- Swallow-only-bound (keyboard-capture.md) re-emits far fewer keys (only remaps and explicit passthrough), shrinking the window to near nothing.
- A real tag. Raw `core-graphics` can set `kCGEventSourceUserData` on the posted event, and the callback reads it back, which is exact and not racy. rdev does not expose event fields, so this means dropping to `core-graphics` for the emit path only. That is Rust but `unsafe` FFI, so `freddie_keyboard` would opt out of the workspace `forbid(unsafe_code)` for that module. This is the "unless we need to write it" case.

## Chords and named keys

- A chord like cmd+r is emitted as a sequence: press `MetaLeft`, press `KeyR`, release `KeyR`, release `MetaLeft`. rdev has no flags-on-a-key-event API, so the modifier is its own key events around the key, and order matters (modifier down before the key, up after).
- Named keys (escape, arrows, function keys) are just their `rdev::Key` variants; nothing special.
- Layout-independent text: rdev is key-code based, so `emit` produces whatever the key maps to under the current layout. Posting arbitrary Unicode without a key code (macOS `CGEventKeyboardSetUnicodeString`) is not exposed by rdev; if we need it, that is another raw `core-graphics` path. For a remapper this rarely matters (we mostly re-emit real keys), so defer it.

## Re-emit (passthrough and remap)

The hijack swallows keys, so anything that should still reach the app is re-emitted:

- A remap swallows the original and emits the replacement (its synthesized key, `Command`, etc.).
- Explicit passthrough (`Passthru` bind) swallows the original and re-emits the same key identically, so the effect has to carry the original key.
- Implicit passthrough (unbound) is passed natively by the tap and never emitted (keyboard-capture.md).

Every re-emit must be counted or tagged so the tap ignores it.

## Modifiers and hold

Simple emits are press-then-release immediately. Anything that holds a modifier across several keys, or holds a key, has to emit the press, do the keys, then the release, tracking state. On the input side this is the same idea as modeling held modifiers as layers (modifier-keys.md).

## Open questions

- Counter guard vs a real `core-graphics` tag: is swallow-only-bound enough to keep the counter safe, or do we need the tag and its `unsafe` FFI?
- Do we ever need Unicode/text emit (raw `core-graphics`), or is key-code re-emit always enough?
- Chord emit as a key sequence: any timing (small sleeps) needed for apps to register the modifier, or is back-to-back fine?
- Does re-emitting a swallowed key preserve everything the app needs (auto-repeat, exact modifiers), or do subtleties leak?
