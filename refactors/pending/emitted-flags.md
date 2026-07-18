# an emitted event is built from a fresh source

`EmitterState` holds one `CGEventSource` for the life of the run, and posting through a source mutates its state. One arrow key leaves `kCGEventFlagMaskNumericPad` set in it, every event built afterwards is born carrying that bit, and `cmd`-`space` stops opening Spotlight until mercury restarts. Building each event from its own source removes the accumulated state, and with it the whole class of bug.

## The failure

`post` keeps whatever the source put in the bits `MODIFIERS` does not name:

```rust
let untouched = event.get_flags() & !MODIFIERS;
event.set_flags(untouched | to_cg(flags));
```

`MODIFIERS` names six bits: `AlphaShift`, `Shift`, `Control`, `Alternate`, `Command`, `SecondaryFn`. It does not name `NumericPad` (`0x0020_0000`), which the arrows and the keypad carry.

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

## Change 1: build each emitted event from its own source

`MODIFIERS` is unchanged and still correct: it clears the modifier bits so the caller's `ModifierFlags` state them outright. What changes is that `untouched` now holds only the bits the key itself carries, because there is no accumulated state left to inherit.

### `crates/freddie_keyboard/src/sys/macos.rs`

Before:

```rust
struct EmitterState {
    source: CGEventSource,
    tag: i64,
}

impl EmitterState {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// A `CGEvent`'s own flags are baked in from the source's state when it is created, which
    /// lags a modifier posted microseconds earlier. So the event states its own modifiers rather
    /// than trusting the source: whoever built it said what it carries, and we apply exactly that.
    fn post(&self, key: Key, down: bool, flags: ModifierFlags) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
            .map_err(|_| EmitError::Post)?;
        let untouched = event.get_flags() & !MODIFIERS;
        event.set_flags(untouched | to_cg(flags));
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}
```

After:

```rust
struct EmitterState {
    tag: i64,
}

impl EmitterState {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// A `CGEvent`'s own flags are baked in from the source's state when it is created, which
    /// lags a modifier posted microseconds earlier. So the event states its own modifiers rather
    /// than trusting the source: whoever built it said what it carries, and we apply exactly that.
    ///
    /// The source is built here, for this one event, and dropped with it. Posting through a source
    /// mutates it: an arrow key leaves `NumericPad` set, and every event built from that source
    /// afterwards is born carrying it, which stops `cmd`-`space` from being the Spotlight hotkey.
    /// A source with no history has nothing to leak, so the event is born with exactly the flags
    /// its own key carries and `untouched` keeps only those.
    fn post(&self, key: Key, down: bool, flags: ModifierFlags) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| EmitError::Post)?;
        let event = CGEvent::new_keyboard_event(source, code, down).map_err(|_| EmitError::Post)?;
        let untouched = event.get_flags() & !MODIFIERS;
        event.set_flags(untouched | to_cg(flags));
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}
```

`Emitter` keeps its `Rc<EmitterState>`, which is what makes it `!Send`; mercury relies on that to keep the emitter on the worker thread that owns it.

`intercept` no longer needs its own source, and `encode` builds one per call for the same reason:

Before:

```rust
    let thread = std::thread::spawn(move || {
        let source = CGEventSource::new(CGEventSourceStateID::Private).ok();
```

```rust
fn encode(source: Option<&CGEventSource>, out: &KeyEvent) -> Option<CGEvent> {
    let code = to_code(out.key)?;
    let down = out.press == PressType::Down;
    CGEvent::new_keyboard_event(source?.clone(), code, down).ok()
}
```

After:

```rust
    let thread = std::thread::spawn(move || {
```

```rust
/// A remapped event, built from a source of its own so it carries only the flags its key
/// carries. See [`EmitterState::post`].
fn encode(out: &KeyEvent) -> Option<CGEvent> {
    let code = to_code(out.key)?;
    let down = out.press == PressType::Down;
    let source = CGEventSource::new(CGEventSourceStateID::Private).ok()?;
    CGEvent::new_keyboard_event(source, code, down).ok()
}
```

Its call site in the tap callback loses the argument:

```rust
                    Decision::Remap(out) => encode(&out)
                        .map_or(CallbackResult::Drop, CallbackResult::Replace),
```

And the emitter is constructed without one:

```rust
    let emitter = Emitter(Rc::new(EmitterState { tag }));
```

### Tests, in the existing `mod tests` in `macos.rs`

The contamination is only observable after a real post, so the unit tests cover the part that is pure, and the hand check below covers the rest.

```rust
// The keys that carry NumericPad are born with it; the emitter must neither add it to
// other keys nor strip it from these. `MODIFIERS` deliberately does not name it, so
// `untouched` carries it, which is correct only when the source has no history.
#[test]
fn a_fresh_source_gives_each_key_its_own_flags() {
    let source = CGEventSource::new(CGEventSourceStateID::Private).expect("a private source");
    let arrow = CGEvent::new_keyboard_event(source.clone(), KeyCode::UP_ARROW, true)
        .expect("an arrow event");
    let space =
        CGEvent::new_keyboard_event(source, KeyCode::SPACE, true).expect("a space event");
    assert!(arrow.get_flags().contains(CGEventFlags::CGEventFlagNumericPad));
    assert!(!space.get_flags().contains(CGEventFlags::CGEventFlagNumericPad));
}

// What the bug produced: a space born with NumericPad from a contaminated source. The mask
// must not be what saves us here, because it does not name that bit; the fresh source is.
#[test]
fn the_mask_does_not_clear_the_keypad_bit() {
    assert!(!MODIFIERS.contains(CGEventFlags::CGEventFlagNumericPad));
}
```

## Change 2: keep the emitter's log line

`post` writes what it actually put on the wire, at `debug` so the log file keeps it. This is what found the bug: the dispatch record showed byte-identical `ModifierFlags` for a working and a failing `cmd`-`space`, because the difference was a raw bit the portable type does not carry.

```rust
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        // What actually goes on the wire, which the portable `KeyEvent` cannot show: the raw flag
        // bits, what was carried over from the source, and the type the OS chose from the keycode
        // (`FlagsChanged` for a modifier, `KeyDown`/`KeyUp` otherwise). At `debug` so the log file
        // keeps it, since two presses that dispatch identically can still post differently.
        tracing::debug!(
            ?key,
            down,
            raw_flags = %format!("{:#010x}", event.get_flags().bits()),
            kept_from_source = %format!("{:#010x}", untouched.bits()),
            kind = ?event.get_type(),
            "post"
        );
        event.post(CGEventTapLocation::Session);
```

Costs a third line per key in the log file, alongside the dispatch record and mercury's `emitted`.

## Verifying by hand

```
cargo run -p mercury
```

- `cmd`-`space` opens Spotlight.
- Press an arrow key.
- `cmd`-`space` opens Spotlight. Before this change it does not, until mercury is restarted.
- Arrow keys still move the cursor, and the log shows `raw_flags=0x00a00000` for them, `NumericPad` and `SecondaryFn` both present.
- `kept_from_source` is `0x00000000` for every key except the arrows and the keypad, for the whole run, however many arrows have been pressed.

## Not covered

A modifier stuck down in the emitted stream is a separate failure with a separate cause, and this change does not address it. The instrumentation above is what would catch it: a modifier bit persisting in `raw_flags` across posts where no modifier is held.
