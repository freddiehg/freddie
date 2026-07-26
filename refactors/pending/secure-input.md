# Secure input: stop interpreting the modifiers the tap still sees

While any process has secure event input enabled (a focused password field), a session `CGEventTap` receives no `KeyDown`/`KeyUp` for ordinary keys; those go straight to the app. `FlagsChanged` still arrives. So a physical Shift+L reaches a consumer as ShiftRight Down, then ShiftRight Up, with nothing between, and figaro's shift dual-role reads that as an alone tap and types `)` into the field beside the app's own Shift+L. Swallowing the `FlagsChanged` does not help: the flags on the hidden keys come from the HID state, not from the tap-filtered stream.

Remapping inside a secure field stays impossible on this backend (`refactors/past/cgevent-vs-hid.md`; the HID upgrade in `refactors/past/hid-backend.md` is what would buy it). What we build instead: `freddie_keyboard` samples the secure-input state once per delivered event and hands it to `on_key`, and figaro mirrors that into the model and passes modifier holds through verbatim while it is active.

## Change 1 (freddie): `freddie_keyboard` reports secure input with each key

Independently shippable: mercury takes the new argument and ignores it, so nothing behaves differently until a consumer reads it.

### The type

`crates/freddie_keyboard/src/lib.rs`, beside `CaptureError`:

```rust
/// Whether some process has secure event input on (macOS: a focused password field).
///
/// While `Active`, the session tap receives no `KeyDown`/`KeyUp` for ordinary keys, only
/// `FlagsChanged`, so a consumer interpreting modifier holds (a dual-role) is seeing a stream
/// with the other keys missing. Sampled once per delivered event: the value describes the
/// session as this event arrived.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecureInput {
    Inactive,
    Active,
}
```

### The sample

`IsSecureEventInputEnabled` is HIToolbox's, inside the Carbon umbrella, and no safe binding exists in `core-graphics` or the `objc2` crates. The crate takes the `freddie_windows` lint arrangement: the workspace `forbid` becomes a crate-local `deny`, and each site carries `#[expect(unsafe_code)]` with a SAFETY comment.

`crates/freddie_keyboard/Cargo.toml`, before:

```toml
# For `rc::autoreleasepool`. `CGEventPost` autoreleases, and an `Emitter` posts from whatever
# thread owns it, which is not required to have a pool. The one function used is safe, so this
# does not cost the crate its `unsafe_code = "forbid"`.
objc2 = "0.6"
```

```toml
[lints]
workspace = true
```

after:

```toml
# For `rc::autoreleasepool`. `CGEventPost` autoreleases, and an `Emitter` posts from whatever
# thread owns it, which is not required to have a pool.
objc2 = "0.6"
```

```toml
# Not `workspace = true`: the workspace forbids `unsafe_code`, and `forbid` cannot be relaxed
# from inside the crate. `IsSecureEventInputEnabled` has no safe binding, so the one extern and
# its call site are unsafe, allowed there with SAFETY comments. Every other lint matches the
# workspace table, which this crate does not inherit.
[lints.rust]
unsafe_code = "deny"
unused = { level = "deny", priority = -1 }

[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery = { level = "deny", priority = -1 }
cargo = { level = "deny", priority = -1 }
multiple_crate_versions = "allow"
cargo_common_metadata = "allow"
mut_mut = "allow"
redundant_pub_crate = "allow"
ignored_unit_patterns = "allow"
missing_const_for_fn = "allow"
empty_structs_with_brackets = "deny"
```

`crates/freddie_keyboard/src/sys/macos.rs`, new items (the `use crate::{CaptureError, EmitError};` line gains `SecureInput`):

```rust
// SAFETY: `IsSecureEventInputEnabled` is exported by HIToolbox inside the Carbon umbrella this
// block links. It takes nothing and returns a `Boolean` (one byte, 0 or 1) read from session
// state.
#[expect(unsafe_code)]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

/// The session's secure-input state as of now, sampled per delivered event in the tap callback.
fn secure_input() -> SecureInput {
    // SAFETY: no arguments, no pointers; reads one session bit.
    #[expect(unsafe_code)]
    if unsafe { IsSecureEventInputEnabled() } != 0 {
        SecureInput::Active
    } else {
        SecureInput::Inactive
    }
}
```

### The plumb

`run_tap`, before:

```rust
fn run_tap(
    on_key: impl FnMut(KeyEvent, &CGEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
```

```rust
                tracing::debug!(?input, source_pid, "tap");
                match decide(&input, on_key.borrow_mut()(input.clone(), event)) {
```

after:

```rust
fn run_tap(
    on_key: impl FnMut(KeyEvent, SecureInput, &CGEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
```

```rust
                let secure = secure_input();
                tracing::debug!(?input, ?secure, source_pid, "tap");
                match decide(&input, on_key.borrow_mut()(input.clone(), secure, event)) {
```

`intercept`, before:

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    run_tap(move |input, _event| on_key(input))
}
```

after (doc comment gains one line: "`on_key` also receives the session's [`SecureInput`] state, sampled as the event arrived."):

```rust
pub fn intercept(
    on_key: impl Fn(KeyEvent, SecureInput) -> Option<KeyEvent> + Send + 'static,
) -> Result<(Interceptor, Emitter), CaptureError> {
    run_tap(move |input, secure, _event| on_key(input, secure))
}
```

`intercept_with_source`, before:

```rust
    F: Fn((KeyEvent, T)) -> Option<KeyEvent> + Send + 'static,
{
    let mut by_source: HashMap<SourceId, T> = HashMap::new();
    run_tap(move |input, event| {
        let class = match source_of(event) {
```

```rust
        on_key((input, class))
    })
}
```

after:

```rust
    F: Fn((KeyEvent, T), SecureInput) -> Option<KeyEvent> + Send + 'static,
{
    let mut by_source: HashMap<SourceId, T> = HashMap::new();
    run_tap(move |input, secure, event| {
        let class = match source_of(event) {
```

```rust
        on_key((input, class), secure)
    })
}
```

### mercury

`crates/mercury/src/daemon.rs`, the closure only; mercury has no dual-roles, so nothing reads the value yet:

```rust
    let grabbed = freddie_keyboard::intercept({
        let event_tx = event_tx.clone();
        move |ev, _secure| {
```

## Change 2 (figaro): the model mirrors secure input and the dual-roles stand down

Lands in the figaro repo. The daemon edge-detects the per-event sample and reports changes as an event; the model records it at the root; `dual_role_gate` reads it.

### The event and trigger

`src/sources.rs`, beside `Displays`/`DisplaysChanged`:

```rust
pub use freddie_keyboard::SecureInput;

/// The daemon's report that the sampled secure-input state changed. Idempotent: it assigns.
#[cfg_attr(feature = "testing", derive(PartialEq))]
#[derive(Debug)]
pub struct SecureInputChanged {
    pub state: SecureInput,
}

/// A trigger matching any secure-input change.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SecureInputs;
impl EventTrigger for SecureInputs {
    type Event = SecureInputChanged;
    fn is_matching(&self, _ev: &SecureInputChanged) -> bool {
        true
    }
}
```

`src/model.rs`: `FigaroTrigger` gains `SecureInputs(SecureInputs)` after `Displays(Displays)`, `FigaroEvent` gains `SecureInput(SecureInputChanged)` after `Displays(DisplaysChanged)`, and both `use crate::{...}` lists gain the two names.

`src/lib.rs`: the `pub use sources::{...}` list gains `SecureInput`, `SecureInputChanged`, `SecureInputs`. The `use crate::{...}` lists in `src/figaro.rs` gain `SecureInput`, `SecureInputChanged`, `SecureInputs`; `src/daemon.rs` gains `SecureInput` and `SecureInputChanged` (from `figaro::`); `tests/transitions.rs` gains `SecureInput` and `SecureInputChanged`.

### The root field and its handler

`src/figaro.rs`, the struct:

```rust
    /// The physical truth about which modifier keys are down, kept current by the
    /// `track_held_modifiers` post on every key in every layer. Entering and leaving typing
    /// reads it to synchronize the app's modifier view. See [`HeldModifiers`].
    pub held: HeldModifiers,
    /// Whether the session is in secure input (a focused password field), reported by the
    /// daemon's per-event sample. `dual_role_gate` reads it. No boot seed: it is read only while
    /// dispatching a key, and any key sampled under a different state is preceded on the channel
    /// by the change event that corrects this field.
    pub secure_input: SecureInput,
```

`Figaro::new` gains `secure_input: SecureInput::Inactive,`. The `#[bind(...)]` block gains one row after `Displays => on_displays_changed,`:

```rust
    SecureInputs => record_secure_input,
```

The handler, in `src/figaro.rs` beside `quit`:

```rust
/// The daemon's secure-input report: assign it. Idempotent, like every state report.
pub(crate) fn record_secure_input<'x>(
    ev: &SecureInputChanged,
    _snap: (),
    st: AscendState<'_, FigaroPath<'x>>,
) -> (Vec<FigaroEffect>, Completed<FigaroPath<'x>>) {
    let root: FigaroPath<'x> = st.state.into_ancestor();
    root.secure_input = ev.state;
    (Vec::new(), root.complete())
}
```

### The daemon

`src/daemon.rs`, the tap closure, before:

```rust
    let grabbed = freddie_keyboard::intercept_with_source(figaro::categorize, {
        let event_tx = event_tx.clone();
        let live = Cell::new(LivePassHeld::default());
        move |(ev, device)| {
            let pass = live_pass(&ev, &live);
```

after (the `Cell` mirrors `live`; the change event goes on the channel before the key that was sampled under it, which is what makes the model's field correct by the time that key dispatches):

```rust
    let grabbed = freddie_keyboard::intercept_with_source(figaro::categorize, {
        let event_tx = event_tx.clone();
        let live = Cell::new(LivePassHeld::default());
        let secure = Cell::new(SecureInput::Inactive);
        move |(ev, device), secure_now| {
            if secure.replace(secure_now) != secure_now {
                let _ = event_tx.send(FigaroEvent::SecureInput(SecureInputChanged {
                    state: secure_now,
                }));
            }
            let pass = live_pass(&ev, &live);
```

### The gate

`src/figaro.rs`, `dual_role_gate`. Two edits.

First, a secure branch at the top of the hold-key arm, before the Escape/shift foreign checks, so it applies uniformly in every layer. While secure input is on, the other key never arrives, so a hold must not arm; a hold that armed before the field took focus was physically a modifier hold and resolves as one. Before:

```rust
        if let Some(i) = self
            .typing_state
            .dual
            .roles
            .iter()
            .position(|r| r.hold() == ev.key.key)
        {
            let hold = self.typing_state.dual.roles[i].hold();
            let already_held = self.typing_state.dual.roles[i].is_held();
```

after:

```rust
        if let Some(i) = self
            .typing_state
            .dual
            .roles
            .iter()
            .position(|r| r.hold() == ev.key.key)
        {
            // Secure input hides the other key from the tap, so the hold cannot be judged
            // alone-or-modifier. A Down does not arm; an Up resolves a Pending hold as the
            // modifier it physically was, releases an AsModifier hold as usual, and otherwise
            // the physical event stands as it arrived.
            if self.secure_input == SecureInput::Active {
                let role = &mut self.typing_state.dual.roles[i];
                let synth = match ev.key.press {
                    PressType::Down => Vec::new(),
                    PressType::Up => {
                        let mut synth = role.promote_if_pending();
                        synth.extend(role.on_hold(PressType::Up));
                        synth
                    }
                };
                return Some(if synth.is_empty() {
                    // The post hook does not run on a gated event, so held is kept here.
                    self.held.apply(&ev.key);
                    vec![emit(ev.key.key, ev.key.press, ev.key.flags)]
                } else {
                    emit_key_events(self, synth)
                });
            }
            let hold = self.typing_state.dual.roles[i].hold();
            let already_held = self.typing_state.dual.roles[i].is_held();
```

Second, an Up whose role is Idle passes verbatim instead of being swallowed. That is the release whose Down was hidden inside a field that has since lost focus, and it also heals a hold that predates the grab. Before:

```rust
            let synth = self.typing_state.dual.roles[i].on_hold(ev.key.press);
            out.extend(emit_key_events(self, synth));
            return Some(out);
```

after:

```rust
            if ev.key.press == PressType::Up && !self.typing_state.dual.roles[i].is_held() {
                // The Down was hidden (a secure field that has since lost focus) or predates
                // the grab: the physical release stands as it arrived.
                self.held.apply(&ev.key);
                out.push(emit(ev.key.key, ev.key.press, ev.key.flags));
                return Some(out);
            }
            let synth = self.typing_state.dual.roles[i].on_hold(ev.key.press);
            out.extend(emit_key_events(self, synth));
            return Some(out);
```

Nothing else changes. Escape needs no special case: it is an ordinary key, hidden entirely while secure input is on, so its dual-role never sees an event to misread. Kinesis RCtrl alone still goes Home from inside a field, deliberately: it is a layer command and types nothing.

### What the user sees

- Typing a capital in a password field: the app's own Shift+L and nothing else. No paren.
- Holding shift before focusing the field and releasing inside it: the app sees a real shift down and up. No paren.
- Pressing shift inside the field and releasing after focus leaves it: the physical release passes verbatim. No paren, no stuck role.
- Everything else inside the field is native passthrough, because the tap never sees those keys: AltIns, jk, and the remaps do not apply there. Unchanged by this doc.

### Tests

`tests/transitions.rs`:

```rust
fn secure(state: SecureInput) -> FigaroEvent {
    FigaroEvent::SecureInput(SecureInputChanged { state })
}

// Inside a password field only the shift's FlagsChanged arrive. The dual-role must not read
// the pair as an alone tap: both halves pass verbatim, carrying the flags they arrived with.
#[test]
fn secure_input_shift_passes_verbatim() {
    let mut m = typing();
    assert_eq!(m.handle(&secure(SecureInput::Active)), Some(vec![]));
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Down,
            ModifierFlags::SHIFT,
            DeviceClass::BuiltIn,
        )),
        Some(vec![emit_with(
            Key::ShiftRight,
            PressType::Down,
            ModifierFlags::SHIFT
        )])
    );
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Up,
            ModifierFlags::empty(),
            DeviceClass::BuiltIn,
        )),
        Some(vec![emit(Key::ShiftRight, PressType::Up)])
    );
}

// A hold armed before the field took focus was physically a modifier hold: its release inside
// the field resolves as shift down + up, not the alone tap.
#[test]
fn secure_input_resolves_a_pending_hold_as_the_modifier() {
    let mut m = typing();
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Down,
            ModifierFlags::SHIFT,
            DeviceClass::BuiltIn,
        )),
        Some(vec![])
    );
    assert_eq!(m.handle(&secure(SecureInput::Active)), Some(vec![]));
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Up,
            ModifierFlags::empty(),
            DeviceClass::BuiltIn,
        )),
        Some(vec![
            emit(Key::ShiftRight, PressType::Down),
            emit(Key::ShiftRight, PressType::Up),
        ])
    );
}

// The Down was hidden inside the field; the field lost focus before the release. The release
// passes verbatim rather than being swallowed as a spurious up.
#[test]
fn secure_input_release_after_leaving_the_field_passes_verbatim() {
    let mut m = typing();
    assert_eq!(m.handle(&secure(SecureInput::Active)), Some(vec![]));
    let _ = m.handle(&key_event(
        Key::ShiftRight,
        PressType::Down,
        ModifierFlags::SHIFT,
        DeviceClass::BuiltIn,
    ));
    assert_eq!(m.handle(&secure(SecureInput::Inactive)), Some(vec![]));
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Up,
            ModifierFlags::empty(),
            DeviceClass::BuiltIn,
        )),
        Some(vec![emit(Key::ShiftRight, PressType::Up)])
    );
}

// Outside secure input nothing changes: shift alone is still the paren.
#[test]
fn secure_input_off_keeps_the_alone_tap() {
    let mut m = typing();
    assert_eq!(m.handle(&secure(SecureInput::Active)), Some(vec![]));
    assert_eq!(m.handle(&secure(SecureInput::Inactive)), Some(vec![]));
    let _ = m.handle(&key_event(
        Key::ShiftRight,
        PressType::Down,
        ModifierFlags::SHIFT,
        DeviceClass::BuiltIn,
    ));
    assert_eq!(
        m.handle(&key_event(
            Key::ShiftRight,
            PressType::Up,
            ModifierFlags::empty(),
            DeviceClass::BuiltIn,
        )),
        Some(vec![
            emit_with(Key::Num0, PressType::Down, ModifierFlags::SHIFT),
            emit_with(Key::Num0, PressType::Up, ModifierFlags::SHIFT),
        ])
    );
}
```
