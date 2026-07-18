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

## Change 1: the tag is a newtype

A prefactor. The tag is a bare `i64` that must round-trip through one specific `CGEvent` field, and nothing type-checks that. Wrapping it puts the field name in one place and gives the read and the write a name.

### `crates/freddie_keyboard/src/sys/macos.rs`

Before:

```rust
fn new_tag() -> i64 {
    // Per-process random, so an interceptor skips only its own emitter's output.
    let mut h = RandomState::new().build_hasher();
    h.write_u8(0);
    i64::from_ne_bytes(h.finish().to_ne_bytes())
}
```

After:

```rust
/// The marker an emitted event carries so the interceptor recognizes its own output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tag(i64);

impl Tag {
    /// Per-process random, so an interceptor skips only its own emitter's output.
    fn new() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(0);
        Self(i64::from_ne_bytes(h.finish().to_ne_bytes()))
    }

    /// Marks `event` as this emitter's, so the tap passes it rather than handling it again.
    fn stamp(self, event: &CGEvent) {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.0);
    }

    /// Whether `event` carries this tag, and so came from this emitter.
    fn marks(self, event: &CGEvent) -> bool {
        event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == self.0
    }
}
```

`EventField::EVENT_SOURCE_USER_DATA` then appears nowhere else. The tap callback reads:

Before:

```rust
                if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == tag {
                    return CallbackResult::Keep; // our own emit
                }
```

After:

```rust
                if tag.marks(event) {
                    return CallbackResult::Keep; // our own emit
                }
```

And `intercept` starts with `let tag = Tag::new();`, with `EmitterState.tag` typed `Tag`.

```rust
#[test]
fn a_tag_marks_only_its_own_events() {
    let source = CGEventSource::new(CGEventSourceStateID::Private).expect("a private source");
    let event =
        CGEvent::new_keyboard_event(source, KeyCode::SPACE, true).expect("a keyboard event");
    let (mine, theirs) = (Tag::new(), Tag(1));
    assert!(!mine.marks(&event));
    mine.stamp(&event);
    assert!(mine.marks(&event));
    assert!(!theirs.marks(&event));
}
```

## Change 2: one constructor owns the source

`CGEventSource` appears exactly once in the file, inside the constructor below, and is never held in a field. A source that outlives one event is then not something to remember to avoid; there is nowhere to put one.

`MODIFIERS` is unchanged and still correct: it clears the modifier bits so the caller's `ModifierFlags` state them outright. What changes is that `untouched` now holds only the bits the key itself carries, because there is no accumulated state left to inherit.

The constructor also fixes a dormant bug in `encode`, which ignores the flags it is handed, so a remapped key carries whatever the source baked in. It never fires today because mercury's callback always returns `None`, but both paths go through one place now, and that place applies the flags.

### `crates/freddie_keyboard/src/sys/macos.rs`

Before:

```rust
fn encode(source: Option<&CGEventSource>, out: &KeyEvent) -> Option<CGEvent> {
    let code = to_code(out.key)?;
    let down = out.press == PressType::Down;
    CGEvent::new_keyboard_event(source?.clone(), code, down).ok()
}
```

After:

```rust
/// A keyboard event for `key`, carrying exactly `flags`, built from a source of its own.
///
/// The source is created here and dropped with the event, and is deliberately not stored
/// anywhere. Posting through a source mutates it: an arrow key leaves `NumericPad` in its
/// state, every event built from it afterwards is born carrying that bit, and posting that
/// event writes the bit back, which reaffirms it. One arrow key would otherwise stop
/// `cmd`-`space` from being the Spotlight hotkey for the rest of the run. A source with no
/// history has nothing to leak, so the event is born with exactly the flags its own key
/// carries and `untouched` keeps only those.
///
/// Not a `NULL` source, which means the shared session state rather than no state, and so
/// inherits bits other processes have left there.
///
/// # Errors
///
/// Returns [`EmitError::Unmappable`] if the key has no code on this OS, and
/// [`EmitError::Post`] if the OS refused to build the source or the event.
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

Its caller in the tap callback loses the source argument, and says so when it drops a key:

```rust
                    Decision::Remap(out) => match keyboard_event(out.key, out.press, out.flags) {
                        Ok(event) => CallbackResult::Replace(event),
                        Err(e) => {
                            tracing::warn!(key = ?out.key, error = %e, "dropped a remapped key");
                            CallbackResult::Drop
                        }
                    },
```

And `intercept` no longer builds a source for the tap thread:

```rust
    let thread = std::thread::spawn(move || {
```

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
    /// The event states its own modifiers rather than trusting a source: whoever built it said
    /// what it carries, and we apply exactly that. See [`keyboard_event`].
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        let event = keyboard_event(key, press, flags)?;
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, self.tag);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}
```

`Emitter::emit` hands the press through rather than flattening it to a bool:

Before:

```rust
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        self.0.post(key, press == PressType::Down, flags)
    }
```

After:

```rust
    pub fn emit(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        self.0.post(key, press, flags)
    }
```

The emitter is constructed without a source:

```rust
    let emitter = Emitter(Rc::new(EmitterState { tag }));
```

`Emitter` keeps its `Rc<EmitterState>`, which is what makes it `!Send`; mercury relies on that to keep the emitter on the worker thread that owns it.

### Tests, in the existing `mod tests` in `macos.rs`

The contamination is only observable after a real post, so the unit tests cover what is pure and the hand check below covers the rest.

```rust
// The keys that carry NumericPad are born with it, and `MODIFIERS` deliberately does not
// name it, so `untouched` carries it through. That is correct only for a source with no
// history, which is what `keyboard_event` guarantees.
#[test]
fn a_keys_own_flags_survive_and_others_do_not_appear() {
    let arrow =
        keyboard_event(Key::UpArrow, PressType::Down, ModifierFlags::empty()).expect("an arrow");
    let space =
        keyboard_event(Key::Space, PressType::Down, ModifierFlags::empty()).expect("a space");
    assert!(arrow.get_flags().contains(CGEventFlags::CGEventFlagNumericPad));
    assert!(!space.get_flags().contains(CGEventFlags::CGEventFlagNumericPad));
}

// What the bug produced: `cmd`-`space` posting 0x00300000 rather than 0x00100000.
#[test]
fn a_chord_carries_its_modifier_and_nothing_else() {
    let space =
        keyboard_event(Key::Space, PressType::Down, ModifierFlags::COMMAND).expect("a space");
    assert_eq!(space.get_flags() & MODIFIERS, CGEventFlags::CGEventFlagCommand);
    assert!(!space.get_flags().contains(CGEventFlags::CGEventFlagNumericPad));
}

// `encode` dropped the flags it was handed; the remap path shares the constructor now.
#[test]
fn a_remapped_key_carries_the_flags_it_was_given() {
    let event =
        keyboard_event(Key::KeyR, PressType::Down, ModifierFlags::COMMAND).expect("a key");
    assert!(event.get_flags().contains(CGEventFlags::CGEventFlagCommand));
}

// A modifier is a `FlagsChanged`, which the OS decides from the keycode; the emitter neither
// asks for that nor needs to.
#[test]
fn a_modifier_posts_as_a_flags_changed() {
    let cmd =
        keyboard_event(Key::MetaLeft, PressType::Down, ModifierFlags::COMMAND).expect("a cmd");
    assert_eq!(cmd.get_type(), CGEventType::FlagsChanged);
    let space =
        keyboard_event(Key::Space, PressType::Down, ModifierFlags::empty()).expect("a space");
    assert_eq!(space.get_type(), CGEventType::KeyDown);
}

#[test]
fn a_key_with_no_code_is_unmappable() {
    assert!(matches!(
        keyboard_event(Key::F24, PressType::Down, ModifierFlags::empty()),
        Err(EmitError::Unmappable(Key::F24))
    ));
}
```

## Change 3: the emitter is `Send`, and is one struct

With the `CGEventSource` gone, nothing in the emitter is thread-hostile: `CGEventSource` is `!Send` because it holds a `NonNull`, while `CFRunLoop` and `i64` are both `Send`, so `Interceptor` already is. The `Rc` is the only thing left manufacturing `!Send`, and it is not there for sharing: the emitter is moved into the effect loop once and borrowed after, and never cloned.

Nothing is put in its place. Thread affinity buys mercury two things, and neither comes from a type marker: the state has a single writer, so no `Mutex`, and effects are performed by one loop in dispatch order, so `MetaLeft` down always reaches the OS before the `Space` carrying its flag. Both follow from one channel with one consumer. `Send` also governs moving rather than sharing, so it was never the marker for either.

`EmitterState` existed to be the thing inside the `Rc`, so it goes with it.

### `crates/freddie_keyboard/src/sys/macos.rs`

Before:

```rust
struct EmitterState {
    tag: Tag,
}

impl EmitterState {
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        ...
    }
}

/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter(Rc<EmitterState>);
```

After:

```rust
/// Synthesizes keys through the interceptor's tag, so they are not re-handled.
pub struct Emitter {
    tag: Tag,
}

impl Emitter {
    /// Post `key` going down or coming up, carrying exactly `flags`.
    ///
    /// The event states its own modifiers rather than trusting a source: whoever built it said
    /// what it carries, and we apply exactly that. See [`keyboard_event`].
    fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
        let event = keyboard_event(key, press, flags)?;
        self.tag.stamp(&event);
        event.post(CGEventTapLocation::Session);
        Ok(())
    }
}
```

`use std::rc::Rc;` goes, `intercept` ends with `let emitter = Emitter { tag };`, and the existing `emit` and `tap` methods join the same `impl` block, calling `self.post(..)` rather than `self.0.post(..)`.

### `crates/mercury/src/main.rs`

Three comments credit the emitter for guarantees it was not providing.

Before:

```rust
//! The worker thread runs the tokio runtime, owns the state and the `!Send`
//! `Emitter`, and runs both the event and effect loops. It is the only place
//! state is mutated, so there is no shared mutable state and no `Mutex`.
```

After:

```rust
//! The worker thread runs the tokio runtime, owns the state and the `Emitter`, and runs both
//! the event and effect loops. It is the only place state is mutated, so there is no shared
//! mutable state and no `Mutex`, and effects are performed in dispatch order by one consumer,
//! so a modifier always reaches the OS before the key carrying its flag.
```

Before:

```rust
/// `intercept` has to be called from here rather than from `main`, because it
/// returns the `Emitter`, the `Emitter` is `!Send` (it holds an `Rc`), and the
/// effect loop uses it. It has to be born on the thread it will live on.
///
/// Which is exactly why this future is `!Send`, and why that is fine: it is
/// `block_on`ed by the worker's current-thread runtime and never crosses a
/// thread.
#[expect(clippy::future_not_send)]
async fn run(
```

After:

```rust
/// `intercept` is called from here rather than from `main` because the tap and the effect loop
/// belong with the state they drive, not because anything it returns is pinned to a thread.
///
/// This future is `!Send` because it holds the `Watcher` from `freddie_app_nav::watch`, and
/// that is fine: it is `block_on`ed by the worker's current-thread runtime and never crosses
/// a thread.
#[expect(clippy::future_not_send)]
async fn run(
```

Before:

```rust
/// The `Emitter` is `!Send` by design, so this future is `!Send`; it runs on the
/// worker thread that created the `Emitter` and never crosses a thread.
#[expect(clippy::future_not_send)]
async fn run_effect_loop(
```

After:

```rust
/// Runs on the worker thread, the one consumer of the effect channel, so effects are performed
/// in the order dispatch produced them.
async fn run_effect_loop(
```

`run` keeps its expectation and only that one. It holds the `Watcher` across await points, and `Watcher` is `!Send` three ways: the `NonNull` inside its `RcBlock`, and the `dyn NSObjectProtocol` token being neither `Send` nor `Sync`. `run_effect_loop` holds no such thing once the emitter is `Send`, which is why its expectation is the one that goes.

## Change 4: keep the emitter's log line

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
