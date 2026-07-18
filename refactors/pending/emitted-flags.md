# an emitted event states its own flags

`EmitterState::post` inherits flag bits from the event source instead of stating them. One arrow key contaminates the source for the rest of the run, so every key mercury emits afterwards carries `kCGEventFlagMaskNumericPad`, and `cmd`-`space` stops opening Spotlight until mercury is restarted.

## The failure

`post` keeps whatever the source put in the bits `MODIFIERS` does not name:

```rust
let untouched = event.get_flags() & !MODIFIERS;
event.set_flags(untouched | to_cg(flags));
```

`MODIFIERS` names six bits: `AlphaShift`, `Shift`, `Control`, `Alternate`, `Command`, `SecondaryFn`. It does not name `NumericPad` (`0x0020_0000`). Posting an arrow key through the source puts that bit into the source's state, and every event built from it afterwards is born with it, so `untouched` carries it onto every emitted key for the life of the run.

Measured, from one run in `~/Library/Logs/mercury/mercury.log`, over 9,965 emitted events:

```
19:15:49.269  post key=KeyT    down=false raw_flags=0x00000000 kept_from_source=0x00000000 kind=KeyUp
19:16:00.454  post key=Backspace down=true raw_flags=0x00000000 kept_from_source=0x00000000 kind=KeyDown
19:16:00.774  post key=UpArrow down=true  raw_flags=0x00a00000 kept_from_source=0x00200000 kind=KeyDown
                                                               ^ set here, and on every post after
```

Before the first arrow, `cmd`-`space` posts as the Spotlight hotkey and Spotlight opens:

```
post key=MetaLeft down=true  raw_flags=0x00100000 kept_from_source=0x00000000 kind=FlagsChanged
post key=Space    down=true  raw_flags=0x00100000 kept_from_source=0x00000000 kind=KeyDown
```

After it, the same keypress posts a bit that the `WindowServer` compares against, because `NumericPad` falls inside `NSEventModifierFlagDeviceIndependentFlagsMask` (`0xFFFF_0000`):

```
post key=MetaLeft down=true  raw_flags=0x00300000 kept_from_source=0x00200000 kind=FlagsChanged
post key=Space    down=true  raw_flags=0x00300000 kept_from_source=0x00200000 kind=KeyDown
```

`0x0030_0000` is not `cmd`-`space`, so no symbolic hotkey matches. Typing is unaffected because apps read the character rather than the exact flag set, which is why this reads as "hotkeys randomly stop working" rather than as a broken keyboard.

Repro: start mercury, `cmd`-`space` opens Spotlight, press any arrow key, `cmd`-`space` never opens Spotlight again until restart.

## What carries `NumericPad`

Every keycode that is born with the bit, enumerated over 0-127 against a fresh source:

```
65, 67, 69, 75, 76, 78, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91, 92   the keypad
123, 124, 125, 126                                                   left, right, down, up arrows
```

The arrows also carry `SecondaryFn` (`0x0080_0000`), which is already correct: `MODIFIERS` names it, so it is stripped and re-derived through `ModifierFlags::FN`. `NumericPad` is the only omission.

`Home`, `End`, `PageUp`, `PageDown`, `ForwardDelete` and the function keys carry `SecondaryFn` and not `NumericPad`, so they need nothing here.

## Change 1: derive the keypad bit from the key

`NumericPad` is a property of which key it is, not of anything the source accumulated, so it is computed from the keycode and never inherited. It stays out of the portable `ModifierFlags`, which describes modifiers the user holds; the keypad bit is neither held nor a modifier.

### `crates/freddie_keyboard/src/sys/macos.rs`

Before:

```rust
/// The device-independent modifier bits. Everything else the OS puts in an event's
/// flags is left as it found it.
const MODIFIERS: CGEventFlags = CGEventFlags::from_bits_truncate(
    CGEventFlags::CGEventFlagAlphaShift.bits()
        | CGEventFlags::CGEventFlagShift.bits()
        | CGEventFlags::CGEventFlagControl.bits()
        | CGEventFlags::CGEventFlagAlternate.bits()
        | CGEventFlags::CGEventFlagCommand.bits()
        | CGEventFlags::CGEventFlagSecondaryFn.bits(),
);
```

After:

```rust
/// The bits an emitted event states for itself, cleared from whatever the source
/// supplied before the event's own are applied.
///
/// `NumericPad` is in here for the same reason the modifiers are: posting an arrow key
/// leaves it set in the source's state, so an event built later is born with it, and a
/// `cmd`-`space` carrying it is not the Spotlight hotkey. It is not a modifier the user
/// holds, so it is derived from the keycode by [`keypad_flag`] rather than from
/// [`ModifierFlags`].
const STATED: CGEventFlags = CGEventFlags::from_bits_truncate(
    CGEventFlags::CGEventFlagAlphaShift.bits()
        | CGEventFlags::CGEventFlagShift.bits()
        | CGEventFlags::CGEventFlagControl.bits()
        | CGEventFlags::CGEventFlagAlternate.bits()
        | CGEventFlags::CGEventFlagCommand.bits()
        | CGEventFlags::CGEventFlagSecondaryFn.bits()
        | CGEventFlags::CGEventFlagNumericPad.bits(),
);

/// The arrows and the keypad, the keycodes macOS marks as numeric-pad keys.
///
/// Enumerated against the OS over every keycode 0-127, rather than assumed: 65-92 are
/// the keypad's digits and operators, 123-126 are the four arrows.
const KEYPAD_CODES: &[CGKeyCode] = &[
    65, 67, 69, 75, 76, 78, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91, 92, 123, 124, 125, 126,
];

/// `NumericPad` for a key that is one, nothing for a key that is not.
fn keypad_flag(code: CGKeyCode) -> CGEventFlags {
    if KEYPAD_CODES.contains(&code) {
        CGEventFlags::CGEventFlagNumericPad
    } else {
        CGEventFlags::empty()
    }
}
```

The two remaining uses of `MODIFIERS` are the mask itself in `post`, replaced below, and nothing else; `from_cg` and `to_cg` go through `FLAG_PAIRS` and are unchanged.

Before:

```rust
    fn post(&self, key: Key, down: bool, flags: ModifierFlags) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
            .map_err(|_| EmitError::Post)?;
        let untouched = event.get_flags() & !MODIFIERS;
        event.set_flags(untouched | to_cg(flags));
```

After:

```rust
    fn post(&self, key: Key, down: bool, flags: ModifierFlags) -> Result<(), EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
            .map_err(|_| EmitError::Post)?;
        let untouched = event.get_flags() & !STATED;
        event.set_flags(untouched | to_cg(flags) | keypad_flag(code));
```

The doc comment above `post` already says the event "states its own modifiers rather than trusting the source". This is what makes that true.

### Tests, in the existing `mod tests` in `macos.rs`

```rust
#[test]
fn the_arrows_and_the_keypad_are_numeric_pad_keys() {
    for code in [KeyCode::UP_ARROW, KeyCode::DOWN_ARROW, KeyCode::LEFT_ARROW, KeyCode::RIGHT_ARROW] {
        assert_eq!(keypad_flag(code), CGEventFlags::CGEventFlagNumericPad);
    }
    // 82 is keypad 0, 76 is keypad enter.
    assert_eq!(keypad_flag(82), CGEventFlags::CGEventFlagNumericPad);
    assert_eq!(keypad_flag(76), CGEventFlags::CGEventFlagNumericPad);
}

#[test]
fn ordinary_keys_are_not() {
    for code in [KeyCode::SPACE, KeyCode::RETURN, KeyCode::ANSI_A, KeyCode::ESCAPE, KeyCode::HOME] {
        assert_eq!(keypad_flag(code), CGEventFlags::empty());
    }
}

// The bug: an emitted `cmd`-`space` must carry Command and nothing else, whatever the
// source has accumulated. `STATED` covering NumericPad is what guarantees it.
#[test]
fn stated_flags_cover_the_keypad_bit() {
    assert!(STATED.contains(CGEventFlags::CGEventFlagNumericPad));
    let contaminated = CGEventFlags::CGEventFlagNumericPad | CGEventFlags::CGEventFlagCommand;
    let untouched = contaminated & !STATED;
    assert_eq!(untouched, CGEventFlags::empty());
    assert_eq!(
        untouched | to_cg(ModifierFlags::COMMAND) | keypad_flag(KeyCode::SPACE),
        CGEventFlags::CGEventFlagCommand
    );
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
- Arrow keys still move the cursor, and the log shows `raw_flags=0x00a00000` for them: `NumericPad` and `SecondaryFn` both present, now derived rather than inherited.
- `kept_from_source` stays `0x00000000` for the whole run.

## Not covered

A modifier stuck down in the emitted stream is a separate failure with a separate cause, and this change does not address it. The instrumentation above is what would catch it: a modifier bit persisting in `raw_flags` across posts where no modifier is held.
