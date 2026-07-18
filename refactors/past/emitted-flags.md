# an emitted event is built from a fresh source

`EmitterState` holds one `CGEventSource` for the life of the run, and posting through a source mutates its state. One arrow key leaves `kCGEventFlagMaskNumericPad` set in it, every event built afterwards is born carrying that bit, and `cmd`-`space` stops opening Spotlight until mercury restarts. Building each event from its own source removes the accumulated state, and with it the whole class of bug.

## The failure

`post` keeps whatever the source put in the bits `MODIFIERS` does not name:

```rust
let untouched = event.get_flags() & !MODIFIERS;
event.set_flags(untouched | to_cg(flags));
```

`MODIFIERS` names six bits: `AlphaShift`, `Shift`, `Control`, `Alternate`, `Command`, `SecondaryFn`. It does not name `NumericPad` (`0x0020_0000`), which the arrows and the keypad carry.

Two steps feed each other. Posting an event updates its source's flag state, and `CGEventCreateKeyboardEvent` seeds a new event's flags from that state, which is what the comment on `post` already describes. So posting an arrow, whose event genuinely carries `NumericPad`, leaves that bit in the source. Every following key then inherits it through `untouched`, and `post` writes it back out, which reaffirms it in the source. Mercury re-poisons its own source on every keystroke.

The state itself is not sticky, and a single post with the bit cleared ends it:

```
clean source, space born:            0x20000000
after an arrow, space born:          0x20200000   source state now carries NumericPad
after posting 'a' mercury-style:     0x20200000   posting it back keeps it alive
after one post with it masked off:   0x20000000   one clean post clears it
```

That is why `SecondaryFn` never caused this despite the arrows carrying it too: `MODIFIERS` names it, so it is stripped before every post and the loop cannot close.

Posting is what contaminates. Creating events without posting them leaves the source clean, which is why this hides from any test that does not post:

```
before posting anything:
  space from long-lived source           0x20000000

after posting an arrow through it:
  space from the SAME source             0x20a00000
  space from a FRESH source              0x20000000
  up arrow from a FRESH source           0x20a00000
```

Measured in `~/Library/Logs/mercury/mercury.log` over one run of 9,965 emitted events, the bit appears on the first arrow key and never clears:

```
19:16:00.454  post key=Backspace down=true  raw_flags=0x00000000 kept_from_source=0x00000000
19:16:00.774  post key=UpArrow   down=true  raw_flags=0x00a00000 kept_from_source=0x00200000
                                                                 ^ set here, and on every post after
```

Before that arrow, `cmd`-`space` posts `0x00100000` and Spotlight opens. After it, the same keypress posts `0x00300000`. `NumericPad` falls inside `NSEventModifierFlagDeviceIndependentFlagsMask` (`0xFFFF_0000`), which is what the `WindowServer` compares a symbolic hotkey against, so nothing matches. Typing is unaffected because apps read the character rather than the exact flag set, which is why this reads as "hotkeys randomly stop working" rather than as a broken keyboard.

Repro: start mercury, `cmd`-`space` opens Spotlight, press any arrow key, `cmd`-`space` never opens Spotlight again until restart.

## Why a fresh source rather than a wider mask

Naming `NumericPad` in `MODIFIERS` and deriving it from a table of keypad keycodes fixes this bit and only this bit. The source accumulates state as a matter of design, so the next bit it latches is the same bug again, and a hardcoded keycode table duplicates knowledge the OS already has.

A source built for one event has nothing to accumulate. The event is then born with exactly the flags that key carries, so an arrow keeps `NumericPad` and a space never gains one, with no table to maintain.

Cost, measured over 10,000 iterations each: an event from a shared source takes 6.6µs, from a fresh source 20.2µs, so 13.6µs more per emitted key. At 100 keys a second, which is faster than autorepeat, that is 0.14% of one core, on the effect loop rather than inside the tap callback.

Not a `NULL` source, which the C API allows and is 132ns. `NULL` means the shared session state rather than no state, so the event inherits whatever the whole login session carries, including bits other processes put there. With the session contaminated it returns `0x20a00000` for a space and `0x20a00000` for an arrow, the same value for both, so it does not even carry the key's own flags. A newly created `Private` source starts empty and reflects only what is posted through it, which is why a source that lives for one event has nothing to inherit.

The long-lived source is mercury's choice, not something `CoreGraphics` asks for: `CGEventCreateKeyboardEvent` takes a source per call. That a source accumulates state, and that posting updates it, is `CoreGraphics`.

## What shipped

Four commits, each standing alone.

`1f6a1dc` wraps the tag in a newtype. `Tag(i64)` with `new`, `stamp`, and `marks`, so `EventField::EVENT_SOURCE_USER_DATA` is named in exactly two methods and nowhere else. The tap callback reads `if tag.marks(event)`; `post` calls `self.tag.stamp(&event)`. A test asserts a tag marks an event it stamped and not one stamped by another, which is the property the whole swallow-and-re-emit design rests on and was previously an untested integer comparison.

`58fea8b` is the fix. `encode` is replaced by:

```rust
fn keyboard_event(key: Key, press: PressType, flags: ModifierFlags) -> Result<CGEvent, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| EmitError::Post)?;
    let event = CGEvent::new_keyboard_event(source, code, press == PressType::Down)
        .map_err(|_| EmitError::Post)?;
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(flags));
    Ok(event)
}
```

`CGEventSource` now appears once in the file, inside that function, and is never stored, so a source outliving one event is not something to remember to avoid. `MODIFIERS` is unchanged: with no accumulated state to inherit, `untouched` holds only the bits the key itself carries. The tap thread and `EmitterState` stopped holding a source, and the remap path warns rather than dropping a key silently. It also fixed a dormant bug: `encode` ignored the flags it was handed, so a remapped key carried whatever the source had baked in. It never fired, because mercury's callback always returns `None`.

`a449b21` drops the `Rc`. With the source gone, nothing in the emitter is thread-hostile, and the `Rc` was not there for sharing: the emitter is moved into the effect loop once, borrowed after, never cloned. `EmitterState` existed only to be the value inside it, so `Emitter` became a plain `{ tag: Tag }`. Nothing replaced the `!Send`. Thread affinity buys mercury a single writer for the state, so no `Mutex`, and one consumer of the effect channel, so effects are performed in dispatch order and a modifier reaches the OS before the key carrying its flag. Both follow from one channel with one consumer, and neither was ever enforced by the marker. `run_effect_loop` lost its `#[expect(clippy::future_not_send)]`; `run` keeps its own, because it holds the `Watcher`, which is `!Send` three ways over: the `NonNull` inside its `RcBlock`, and the `dyn NSObjectProtocol` token being neither `Send` nor `Sync`. Four comments in mercury that credited the emitter for those guarantees now name what actually provides them.

`8738e6c` logs what the emitter puts on the wire, at `debug` so the log file keeps it:

```
post key=MetaLeft press=Down raw_flags=0x00100000 kept_from_source=0x00000000 kind=FlagsChanged
post key=Space    press=Down raw_flags=0x00100000 kept_from_source=0x00000000 kind=KeyDown
```

This is what found the bug. The dispatch record showed byte-identical `ModifierFlags` across a working and a failing `cmd`-`space`, because the difference was a raw bit the portable type does not carry. It costs a third line per key in the log file.

## Verified by hand

The only check that reaches this, since the contamination exists only after a real post: mercury running, `cmd`-`space` opens Spotlight, arrow key, `cmd`-`space` opens Spotlight again. Confirmed after `58fea8b`.

The unit tests cover what is pure. `a_chord_carries_its_modifier_and_nothing_else` is the bug pinned: a `cmd`-`space` carries exactly `CGEventFlagCommand`, which is `0x00100000` against the `0x00300000` the log showed.

## Not covered

A modifier stuck down in the emitted stream is a separate failure with a separate cause. The instrumentation is what would catch it: a modifier bit persisting in `raw_flags` across posts where no modifier is held.
