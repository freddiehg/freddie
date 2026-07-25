# one private event source for the process

`keyboard_event` creates a `CGEventSourceStateID::Private` on every emit and every remap. Each `CGEventSourceCreate(Private)` maps about 16KB of process-private shared memory. `CFRelease` on the source drops the CF object; the mapping stays until process exit. There is no reclaim API.

Measured on a ~5h mercury run: `post` count and `shared memory` region count tracked 1:1 (~60k posts, ~60k regions, ~934MB resident SHM). Heap (`MALLOC`) stayed ~45MB. RSS is baseline plus posts × 16KB, unbounded.

The flags on the wire must stay exact: portable modifiers from the caller, plus the non-modifier bits the keycode itself carries (`NumericPad`, on the four arrows and on the keypad). They must not inherit leftover bits from earlier posts through the same source.

## Shape after

Two long-lived private sources, one per thread that builds events:

- the effect loop's `Emitter` holds one
- the tap thread holds one for `Decision::Remap`

Each is created once, when that thread starts building events. Neither is shared across threads (posting mutates source state). Fixed cost: two sources ≈ 32KB for the life of the process. If either cannot be created, `run_tap` fails with `CaptureError`, the same way a tap that cannot install does.

`keyboard_event` takes a borrow of the source. It never calls `CGEventSource::new`. Flags are set to exactly `to_cg(flags) | intrinsic_flags(code)`, never merged with `event.get_flags() & !MODIFIERS`.

```rust
/// The keycodes a clean private source puts `NumericPad` on: the four arrows and the keypad.
///
/// Measured by building an event for every code from 0 to 127 on a clean private source and
/// reading the bit back. `HOME`, `END`, `PAGE_UP` and `PAGE_DOWN` are not in it, and neither
/// are `ANSI_KEYPAD_CLEAR` or `JIS_KEYPAD_COMMA`; the other seventeen keypad codes are.
///
/// By keycode rather than by [`Key`], because the keypad has no `Key` variant. Those keys
/// reach the emitter as `Key::Raw(code)`, so a match on variants would drop the bit for
/// exactly the keys that carry it most: the log for one run has `Raw(76)`
/// (`ANSI_KEYPAD_ENTER`) posted with `kept_from_source=0x00200000`.
const NUMERIC_PAD_CODES: &[CGKeyCode] = &[
    KeyCode::LEFT_ARROW,
    KeyCode::RIGHT_ARROW,
    KeyCode::DOWN_ARROW,
    KeyCode::UP_ARROW,
    KeyCode::ANSI_KEYPAD_DECIMAL,
    KeyCode::ANSI_KEYPAD_MULTIPLY,
    KeyCode::ANSI_KEYPAD_PLUS,
    KeyCode::ANSI_KEYPAD_DIVIDE,
    KeyCode::ANSI_KEYPAD_ENTER,
    KeyCode::ANSI_KEYPAD_MINUS,
    KeyCode::ANSI_KEYPAD_EQUAL,
    KeyCode::ANSI_KEYPAD_0,
    KeyCode::ANSI_KEYPAD_1,
    KeyCode::ANSI_KEYPAD_2,
    KeyCode::ANSI_KEYPAD_3,
    KeyCode::ANSI_KEYPAD_4,
    KeyCode::ANSI_KEYPAD_5,
    KeyCode::ANSI_KEYPAD_6,
    KeyCode::ANSI_KEYPAD_7,
    KeyCode::ANSI_KEYPAD_8,
    KeyCode::ANSI_KEYPAD_9,
];

/// Non-modifier flag bits this keycode carries on a clean private source.
///
/// Posting updates the source's flag state, and `CGEventCreateKeyboardEvent` seeds a new
/// event's flags from that state. Reading those birth flags back (`get_flags() & !MODIFIERS`)
/// and writing them out would re-poison every later key after an arrow (NumericPad on
/// `cmd`-`space`, Spotlight dead until restart). So the bits a key is allowed to keep are
/// named here, and the event's flags are set to exactly those plus the portable modifiers.
///
/// `SecondaryFn` is already in `MODIFIERS` and only reappears when the portable flags carry
/// `FN`.
fn intrinsic_flags(code: CGKeyCode) -> CGEventFlags {
    if NUMERIC_PAD_CODES.contains(&code) {
        CGEventFlags::CGEventFlagNumericPad
    } else {
        CGEventFlags::empty()
    }
}

/// A keyboard event for `key`, carrying exactly `flags`, built from a long-lived private source.
///
/// `source` is the caller's private source for this thread: the emitter's, or the tap's remap
/// source. Creating a new `Private` source per call maps ~16KB of shared memory that
/// `CFRelease` never unmaps, so the source is passed in rather than built here.
///
/// Flags on the wire are `to_cg(flags) | intrinsic_flags(code)` only. Birth flags from the
/// source are ignored: they hold whatever the last post left in the source, not what this key
/// is.
///
/// Not a `NULL` source. `NULL` is the shared session state, and the event inherits bits other
/// processes have left there.
///
/// # Errors
///
/// Returns [`EmitError::Unmappable`] if the key has no code on this OS, and [`EmitError::Post`]
/// if the OS refused to build the event.
fn keyboard_event(
    source: &CGEventSource,
    key: Key,
    press: PressType,
    flags: ModifierFlags,
) -> Result<CGEvent, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    // `new_keyboard_event` takes the source by value; the clone is a CFRetain of the same
    // source, not a second mapping.
    let event = CGEvent::new_keyboard_event(source.clone(), code, press == PressType::Down)
        .map_err(|_| EmitError::Post)?;
    let intrinsic = intrinsic_flags(code);
    event.set_flags(to_cg(flags) | intrinsic);
    tracing::debug!(
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        intrinsic = %format!("{:#010x}", intrinsic.bits()),
        kind = ?event.get_type(),
        "post"
    );
    Ok(event)
}
```

`Emitter` after:

```rust
/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter {
    tag: Tag,
    /// One private source for every event this emitter posts. Created once in [`run_tap`];
    /// posting mutates it, and [`keyboard_event`] ignores birth flags so that mutation cannot
    /// reach the wire.
    source: CGEventSource,
}

impl Emitter {
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        let event = keyboard_event(&self.source, key, press, flags)?;
        self.tag.stamp(&event);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }

    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        self.post(key, press, flags)
    }

    pub fn tap(&self, key: Key, flags: ModifierFlags) -> Result<(), EmitError> {
        self.emit(key, PressType::Down, flags)?;
        self.emit(key, PressType::Up, flags)
    }
}
```

## Change 1: set flags without reading the source

File: `crates/freddie_keyboard/src/sys/macos.rs`.

Still builds a private source per call. Stops the inheritance path that made a long-lived source poison `cmd`-`space`. After this change, a long-lived source is safe; the leak is unchanged until Change 2.

### `keyboard_event` before

```rust
fn keyboard_event(key: Key, press: PressType, flags: ModifierFlags) -> Result<CGEvent, EmitError> {
    let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| EmitError::Post)?;
    let event = CGEvent::new_keyboard_event(source, code, press == PressType::Down)
        .map_err(|_| EmitError::Post)?;
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(flags));
    tracing::debug!(
        ?key,
        ?press,
        raw_flags = %format!("{:#010x}", event.get_flags().bits()),
        kept_from_source = %format!("{:#010x}", untouched.bits()),
        kind = ?event.get_type(),
        "post"
    );
    Ok(event)
}
```

### `keyboard_event` after Change 1

Body is the final one in "Shape after" (`intrinsic_flags` + `keyboard_event(source, ...)` with exact flags). Production call sites still build a throwaway `Private` source and pass `&source`; Change 2 only moves those `new`s to construction time.

Remap path before:

```rust
Decision::Remap(out) => match keyboard_event(out.key, out.press, out.flags) {
    Ok(event) => CallbackResult::Replace(event),
    Err(e) => {
        tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
        CallbackResult::Drop
    }
},
```

Remap path after Change 1 (throwaway source still):

```rust
Decision::Remap(out) => {
    match CGEventSource::new(CGEventSourceStateID::Private) {
        Ok(source) => match keyboard_event(&source, out.key, out.press, out.flags) {
            Ok(event) => CallbackResult::Replace(event),
            Err(e) => {
                tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
                CallbackResult::Drop
            }
        },
        Err(()) => {
            tracing::warn!(key = ?out.key, "dropped a remapped key; no event source");
            CallbackResult::Drop
        }
    }
}
```

`Emitter::post` after Change 1:

```rust
fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| EmitError::Post)?;
    let event = keyboard_event(&source, key, press, flags)?;
    self.tag.stamp(&event);
    event.post(CGEventTapLocation::Session);
    Ok(())
}
```

### Tests after Change 1

Helper:

```rust
fn private_source() -> CGEventSource {
    CGEventSource::new(CGEventSourceStateID::Private).expect("a private source")
}
```

Every existing `keyboard_event(key, press, flags)` call becomes `keyboard_event(&private_source(), key, press, flags)`. Expectations unchanged:

- `a_chord_carries_its_modifier_and_nothing_else`: space + COMMAND has Command, not NumericPad
- `a_keys_own_flags_survive_and_others_do_not_appear`: arrow has NumericPad, space does not
- `a_remapped_key_carries_the_flags_it_was_given`: KeyR + COMMAND has Command

New tests:

```rust
#[test]
fn only_the_arrows_and_the_keypad_are_intrinsically_numeric_pad() {
    for code in [
        KeyCode::UP_ARROW,
        KeyCode::LEFT_ARROW,
        KeyCode::ANSI_KEYPAD_ENTER,
        KeyCode::ANSI_KEYPAD_7,
    ] {
        assert_eq!(
            intrinsic_flags(code),
            CGEventFlags::CGEventFlagNumericPad,
            "{code} carries NumericPad on a clean source"
        );
    }
    // The navigation keys that read like the arrows and are not: measured, not assumed.
    for code in [
        KeyCode::HOME,
        KeyCode::END,
        KeyCode::PAGE_UP,
        KeyCode::PAGE_DOWN,
        KeyCode::ANSI_KEYPAD_CLEAR,
        KeyCode::SPACE,
        KeyCode::ANSI_A,
    ] {
        assert_eq!(
            intrinsic_flags(code),
            CGEventFlags::empty(),
            "{code} carries nothing outside MODIFIERS"
        );
    }
}

// The whole point of the change: what goes on the wire is a function of the key and the
// portable flags, and of nothing the source is holding. Exact equality, so a reintroduced
// `| (get_flags() & !MODIFIERS)` fails here rather than in Spotlight six hours later.
#[test]
fn the_wire_flags_are_the_portable_ones_plus_the_intrinsic_ones() {
    let source = private_source();
    for (key, code, flags) in [
        (Key::Space, KeyCode::SPACE, ModifierFlags::COMMAND),
        (Key::UpArrow, KeyCode::UP_ARROW, ModifierFlags::empty()),
        (Key::UpArrow, KeyCode::UP_ARROW, ModifierFlags::COMMAND),
        (Key::KeyR, KeyCode::ANSI_R, ModifierFlags::CONTROL),
    ] {
        let event =
            keyboard_event(&source, key, PressType::Down, flags).expect("an event for the key");
        assert_eq!(
            event.get_flags(),
            to_cg(flags) | intrinsic_flags(code),
            "{key:?} with {flags:?}"
        );
    }
}
```

The source's accumulated state is not asserted against. Contamination only exists after a real `CGEventPost`, so reaching it from a test means injecting a key into whatever is focused on every `cargo test` and every pre-commit run, and injecting nothing at all in CI, where `cargo test --all --all-features` runs on a `macos-latest` runner with no Accessibility grant. A posted key-up does not contaminate the source (measured: a space born after one still reads `0x20000000`), so the harmless half of the post is also the half that proves nothing; only a key-down does it. `refactors/past/emitted-flags.md` reached the same conclusion, and its check is the manual one under "Verification" below.

The tests above are what makes a long-lived source safe: they pin the output to `to_cg(flags) | intrinsic_flags(code)` exactly, which holds whatever the source accumulates.

## Change 2: create each source once

File: `crates/freddie_keyboard/src/sys/macos.rs`. Depends on Change 1.

### Tap thread

Inside `run_tap`'s spawned thread, before `CGEventTap::with_enabled`, create the remap source once and capture it in the callback. If it cannot be created, the thread sends `Err(())` on the ready channel and returns before the tap installs, so `run_tap` fails with `CaptureError` through the same path an uninstallable tap takes.

Before (after Change 1): each remap calls `CGEventSource::new`.

After:

```rust
let thread = std::thread::spawn(move || {
    let Ok(remap_source) = CGEventSource::new(CGEventSourceStateID::Private) else {
        let _ = signal.send(Err(()));
        return;
    };
    let outcome = CGEventTap::with_enabled(
        // ... same location, placement, options, event types ...
        |_proxy, kind, event| {
            // ... tag / press / code / input unchanged ...
            match decide(&input, on_key.borrow_mut()(input.clone(), event)) {
                Decision::Pass => CallbackResult::Keep,
                Decision::Drop => CallbackResult::Drop,
                Decision::Remap(out) => {
                    match keyboard_event(&remap_source, out.key, out.press, out.flags) {
                        Ok(event) => CallbackResult::Replace(event),
                        Err(e) => {
                            tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
                            CallbackResult::Drop
                        }
                    }
                }
            }
        },
        || {
            let _ = ready_tx.send(Ok(CFRunLoop::get_current()));
            CFRunLoop::run_current();
        },
    );
    if outcome.is_err() {
        let _ = signal.send(Err(()));
    }
});
```

### Emitter

After the tap is ready, create the emitter's source once. Failure to create it is `CaptureError`: an `Emitter` that cannot build events has no working `emit`, and `run_tap` returns both halves or neither.

Before (end of `run_tap`):

```rust
let emitter = Emitter { tag };
Ok((interceptor, emitter))
```

After:

```rust
let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| CaptureError)?;
let emitter = Emitter { tag, source };
Ok((interceptor, emitter))
```

`Emitter::post` after:

```rust
fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
    let event = keyboard_event(&self.source, key, press, flags)?;
    self.tag.stamp(&event);
    event.post(CGEventTapLocation::Session);
    Ok(())
}
```

Outside tests, `CGEventSource::new` appears in exactly two places after this change: once in the tap thread for remaps, once when building the `Emitter`. Not inside `keyboard_event`. Tests still build throwaway sources (`private_source()`, and the tag test's inline one).

## Call sites

Only `crates/freddie_keyboard/src/sys/macos.rs`. Mercury calls `Emitter::emit` / `Emitter::tap`; those signatures do not change. `Emitter` gains a `CGEventSource` field, and `CGEventSource` is not `Send`, so `Emitter` stops being `Send`; mercury still works because the daemon calls `intercept` and drains the effect channel on the same worker thread, under a current-thread runtime's `block_on`, so nothing moves the emitter anywhere.

The workspace denies `clippy::nursery`, which includes `future_not_send`, so the two futures that own the emitter say why they are not `Send`: `#[expect(clippy::future_not_send)]` on `serve` and on `run_effect_loop` in `crates/mercury/src/daemon.rs`. No other mercury, daemon, or effect changes.

## Verification

Automated:

```
cargo test -p freddie_keyboard
```

Includes the `intrinsic_flags` table test and the exact-wire-flags test.

By hand, after `mercury restart` (old process cannot unmap what it already mapped):

1. Flags: `cmd`-`space` opens Spotlight, press any arrow, `cmd`-`space` opens Spotlight again. This is the only check that reaches the contamination, because it needs a real post.
2. Keypad, if the keyboard has one: a keypad digit still types its digit, and the `post` line for it shows `intrinsic=0x00200000` rather than `0x00000000`.
3. Memory: note SHM region count (`vmmap -summary $(pgrep -f 'mercury daemon') | grep 'shared memory'`). Type for a minute (hundreds of posts). Region count stays within a few regions of the baseline; it does not climb one per `post`. RSS stays flat aside from normal heap noise.
4. Log: `post` lines show `intrinsic=0x00200000` on arrows and `intrinsic=0x00000000` on letters/space; `raw_flags` for `cmd`-`space` is `0x00100000` after arrows.

## Ordered commits

1. Change 1: `intrinsic_flags`, `keyboard_event(&CGEventSource, ...)`, stop merging birth flags, tests.
2. Change 2: one source on the tap, one on `Emitter`; `CGEventSource::new` only at those two sites outside tests.
