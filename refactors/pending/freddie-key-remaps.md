# Caps dual-role + number-row remaps

## What this builds

1. `freddie_key_remaps`: shareable pure key remaps over `freddie_keys` only. First export: caps dual-role (alone → Escape, with other keys → Control). README states purpose and the common pieces that belong here.
2. Figaro: `TypingState.caps` is that struct; **every** BuiltIn key while caps is held goes through it (not only the root catch-all), then root / number handlers stop carrying free caps fns.
3. Figaro: `NumberRemaps` binds only the number row (`1`..`0` bare/shift invert → `!@#$%^&*()`). Brackets and backslash move to `SymbolRemaps`.

## Stack after

```text
Figaro
  └─ NumberRemaps          // digits only
       └─ SymbolRemaps     // brackets ↔ cmd+bracket, \ ↔ |
            └─ WisprRemaps
                 └─ Layer
```

`TypingState` holds `caps: CapsAsControl` (from `freddie_key_remaps`) and still holds `shift_alone` (figaro-local for now; same shape, later move if wanted).

## Already elsewhere (do not move into this crate)

- `KeySequence` (`jk` chord, and any ordered bare-key run) lives in `freddie` (`crates/freddie/src/sequence.rs`). It needs optional `TimerGuard` for the inter-key window, so it stays with the timer primitives. Document that in the README; do not duplicate it under `freddie_key_remaps`.

---

# Step 1 — `freddie_key_remaps` crate

Independently shippable: crate + unit tests + README. No figaro.

## Cargo

`crates/freddie_key_remaps/Cargo.toml`:

```toml
[package]
name = "freddie_key_remaps"
description = "Shareable pure key remaps and dual-role state machines over freddie_keys."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
freddie_keys = { path = "../freddie_keys", version = "0.0.1" }

[lints]
workspace = true
```

Workspace `Cargo.toml` members: add `"crates/freddie_key_remaps"`.

## README

`crates/freddie_key_remaps/README.md` — written when the crate is created, not a stub. Shape:

```markdown
# freddie_key_remaps

Shareable, pure keyboard remaps for freddie consumers (mercury, figaro, anything else on
`freddie_keys`). No effects, no timers, no bind, no OS. You own a small state machine on
your model, feed it `KeyEvent`s, and emit what it returns.

## Why this crate

Dual-role caps, shift-alone parens, number-row invert, and similar policies show up in every
remapper. Putting them behind a pure API means each app does not re-implement the state
machine, and tests of the policy do not need a full dispatch tree.

## What belongs here

- Dual-role / alone-vs-held keys (`CapsAsControl`, later shift-alone → paren, etc.)
- Stateless pure rewrites (`KeyEvent` in → optional `KeyEvent` out): number-row invert,
  bracket ↔ cmd+bracket, layout letter maps, …
- Anything whose whole contract is "keys and flags in, keys and flags out," with state only
  on a small owned struct.

## What does not

- Ordered chords with a timeout (`jk` → leave typing): that is `freddie::KeySequence`
  (it needs `TimerGuard`).
- Emitting, grabbing, or posting keys: `freddie_keyboard`.
- Bindings and the state tree: `bind` / `laserbeam` / the app.

## Current exports

- `CapsAsControl` — CapsLock alone → Escape; held with another key → Control.

## Consumer sketch

```rust
// field on your root typing state
caps: CapsAsControl,

// CapsLock event
for ev in caps.on_caps(press) { emit(ev); }

// any other key you are about to emit (or that should promote control)
for ev in caps.promote_if_pending() { emit(ev); }
let flags = caps.stamp(key.flags);
emit(KeyEvent { flags, ..key });
```
```

Keep the README short and concrete. As exports land, list them under "Current exports"; as common candidates sit unbuilt, a one-line "Candidates" list is fine (shift-alone paren, number-row invert) without designing them.

## Types

`crates/freddie_key_remaps/src/lib.rs`:

```rust
//! Shareable pure keyboard remaps: dual-role state machines and flag rewrites over [`freddie_keys`].
//!
//! No effects, no timers, no bind. A consumer feeds [`KeyEvent`]s, gets back events to emit
//! and flags to stamp. State lives in the struct the consumer owns on its root model.
//!
//! Ordered timed chords (`jk`) live in [`freddie::KeySequence`], not here.

mod caps;

pub use caps::CapsAsControl;
```

`crates/freddie_key_remaps/src/caps.rs`:

```rust
use freddie_keys::{Key, KeyEvent, ModifierFlags, PressType};

/// CapsLock dual-role: alone (down then up, no other key) → Escape; other key while held → Control.
///
/// No timer. Alone vs control is only whether another key arrived before release.
///
/// ```text
/// Idle  --caps down-->  Pending
/// Pending --caps up-->  Idle, emit Escape down+up
/// Pending --other key prepare-->  AsControl, emit ControlLeft down
/// AsControl --other key prepare-->  AsControl, stamp CONTROL on flags
/// AsControl --caps up-->  Idle, emit ControlLeft up
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CapsAsControl {
    role: Role,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Role {
    #[default]
    Idle,
    /// Caps is down; no other key yet. Up → Escape.
    Pending,
    /// Another key arrived while caps was down; ControlLeft down has been emitted.
    AsControl,
}

impl CapsAsControl {
    #[must_use]
    pub const fn new() -> Self {
        Self { role: Role::Idle }
    }

    /// Whether Control is live (another key already promoted this hold).
    #[must_use]
    pub const fn is_control(self) -> bool {
        matches!(self.role, Role::AsControl)
    }

    /// CapsLock down or up. Returns the synthetic events to emit; the physical CapsLock is never
    /// re-emitted (consumer swallows it).
    ///
    /// - Down: enter Pending, emit nothing.
    /// - Up while Pending: Escape down+up, return Idle.
    /// - Up while AsControl: ControlLeft up, return Idle.
    /// - Up while Idle: nothing (spurious).
    pub fn on_caps(&mut self, press: PressType) -> Vec<KeyEvent> {
        match press {
            PressType::Down => {
                self.role = Role::Pending;
                Vec::new()
            }
            PressType::Up => match std::mem::replace(&mut self.role, Role::Idle) {
                Role::Pending => vec![
                    KeyEvent {
                        key: Key::Escape,
                        press: PressType::Down,
                        flags: ModifierFlags::empty(),
                    },
                    KeyEvent {
                        key: Key::Escape,
                        press: PressType::Up,
                        flags: ModifierFlags::empty(),
                    },
                ],
                Role::AsControl => vec![KeyEvent {
                    key: Key::ControlLeft,
                    press: PressType::Up,
                    flags: ModifierFlags::empty(),
                }],
                Role::Idle => Vec::new(),
            },
        }
    }

    /// Call before emitting any non-caps key while this dual-role is in force for the device.
    ///
    /// If Pending, promotes to AsControl and returns ControlLeft down (emit first). Otherwise empty.
    pub fn promote_if_pending(&mut self) -> Vec<KeyEvent> {
        if self.role != Role::Pending {
            return Vec::new();
        }
        self.role = Role::AsControl;
        vec![KeyEvent {
            key: Key::ControlLeft,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        }]
    }

    /// Or CONTROL onto `flags` when this hold is acting as control.
    #[must_use]
    pub const fn stamp(self, flags: ModifierFlags) -> ModifierFlags {
        if matches!(self.role, Role::AsControl) {
            flags.union(ModifierFlags::CONTROL)
        } else {
            flags
        }
    }
}
```

`ModifierFlags::union` does not exist today. Use `flags | ModifierFlags::CONTROL` (BitOr is already impl'd) and drop `const` on `stamp` if needed:

```rust
#[must_use]
pub fn stamp(self, flags: ModifierFlags) -> ModifierFlags {
    if matches!(self.role, Role::AsControl) {
        flags | ModifierFlags::CONTROL
    } else {
        flags
    }
}
```

## Unit tests (crate)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn down(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            press: PressType::Down,
            flags: ModifierFlags::empty(),
        }
    }

    #[test]
    fn alone_is_escape() {
        let mut c = CapsAsControl::new();
        assert!(c.on_caps(PressType::Down).is_empty());
        let out = c.on_caps(PressType::Up);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, Key::Escape);
        assert_eq!(out[0].press, PressType::Down);
        assert_eq!(out[1].key, Key::Escape);
        assert_eq!(out[1].press, PressType::Up);
        assert!(!c.is_control());
    }

    #[test]
    fn with_other_key_is_control() {
        let mut c = CapsAsControl::new();
        assert!(c.on_caps(PressType::Down).is_empty());
        let prefix = c.promote_if_pending();
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix[0].key, Key::ControlLeft);
        assert_eq!(prefix[0].press, PressType::Down);
        assert!(c.is_control());
        let flags = c.stamp(ModifierFlags::empty());
        assert!(flags.contains(ModifierFlags::CONTROL));
        // second key: no second promote
        assert!(c.promote_if_pending().is_empty());
        let release = c.on_caps(PressType::Up);
        assert_eq!(release.len(), 1);
        assert_eq!(release[0].key, Key::ControlLeft);
        assert_eq!(release[0].press, PressType::Up);
    }

    #[test]
    fn idle_up_is_noop() {
        let mut c = CapsAsControl::new();
        assert!(c.on_caps(PressType::Up).is_empty());
    }
}
```

## Consumer contract

- Physical CapsLock is never re-emitted. frebbie_keyboard already maps CapsLock → AlphaShift in `flag_for` so taps can drop OS lock; that stays in `freddie_keyboard`, not this crate.
- Synthetic ControlLeft down/up from promote/release are real emits. The consumer updates its own held-modifier mirror when those events go out (figaro: `HeldModifiers::apply`).
- Leaf bindings that claim a key (number invert) must call `promote_if_pending` + `stamp` before emit, same as the root passthrough path. The dual-role does not see the tree; every emit path that can fire while caps is held opts in.

---

# Step 2 — figaro consumes `CapsAsControl`

Independently shippable after step 1.

## Live gap this step closes

Today promote runs only in the root catch-all and the number leaf. A Home binding (`t` → typing) while caps is Pending never hits those paths: caps stays Pending, the layer changes, and promote fires on a later unbound up. That is wrong for "held with anything else → control."

Contract while `caps` is Pending or AsControl on BuiltIn:

- CapsLock itself: `on_caps` only (never re-emitted).
- Any other key: promote if needed, then **emit that key with CONTROL stamped** — do not run the layer binding as a mode command. Holding caps+`t` is ctrl+t, not "enter typing."

So dual-role must sit in front of layer dispatch for BuiltIn keys, not only on unbound fallthrough.

## Dependency

`figaro/Cargo.toml`:

```toml
freddie_key_remaps = { path = "../freddie/crates/freddie_key_remaps" }
```

## State

Before (`state/mod.rs`):

```rust
pub struct TypingState {
    pub held: HeldModifiers,
    pub jk: KeySequence,
    pub caps: CapsRole,
    pub shift_alone: ShiftAlone,
}

pub enum CapsRole {
    Idle,
    Pending,
    AsControl,
}
```

After:

```rust
use freddie_key_remaps::CapsAsControl;

pub struct TypingState {
    pub held: HeldModifiers,
    pub jk: KeySequence,
    pub caps: CapsAsControl,
    pub shift_alone: ShiftAlone,
}

// CapsRole deleted. CapsAsControl is the field type.
```

`Default`: `caps: CapsAsControl::new()`.

Re-exports: drop `CapsRole` from `lib.rs` / `state/mod.rs` pubs.

## Helper: KeyEvent → effect + held

```rust
fn emit_key_events(root: &mut Figaro, events: Vec<KeyEvent>) -> Vec<FigaroEffect> {
    events
        .into_iter()
        .map(|ev| {
            if ev.key.is_modifier() {
                root.typing_state.held.apply(&ev);
            }
            emit(ev.key, ev.press, ev.flags)
        })
        .collect()
}
```

## Where dual-role runs (before layer binds)

Figaro must see BuiltIn keys for dual-role **before** Home/Nav/etc. claim them. Two equivalent shapes; pick the one that matches existing bind hooks when implementing:

### Shape A — gate at the start of `Figaro` / root key handling

Whatever path every `DeviceKey` already enters (root catch-all is not enough; need something that runs for bound keys too). If dispatch has no "always" hook, add a single root bind that matches more broadly, or a thin wrapper in the daemon→dispatch site that is still pure `(state, event) → effects` by calling a method on `Figaro`:

```rust
// On Figaro, called from the same place every Key is dispatched (e.g. before bind::dispatch,
// or as the first thing a root-level DeviceKey handler does when it can short-circuit).
impl Figaro {
    /// BuiltIn dual-role gate. `Some(effects)` means the key is fully handled (caps swallowed,
    /// or control-chord emitted); `None` means leave the event to normal bind dispatch.
    pub fn dual_role_gate(&mut self, ev: &DeviceKey) -> Option<Vec<FigaroEffect>> {
        if ev.device != DeviceClass::BuiltIn {
            return None;
        }
        if ev.key.key == Key::CapsLock {
            return Some(emit_key_events(self, self.typing_state.caps.on_caps(ev.key.press)));
        }
        // While control is live or about to promote: do not let layer bindings steal the key.
        if self.typing_state.caps.is_control()
            || /* pending will promote */ true && matches pending
        {
            // Only short-circuit when Pending or AsControl; Idle falls through.
        }
        ...
    }
}
```

Concrete body when Pending or AsControl:

```rust
if ev.key.key == Key::CapsLock {
    return Some(emit_key_events(self, self.typing_state.caps.on_caps(ev.key.press)));
}
if ev.device == DeviceClass::BuiltIn && !ev.key.key.is_modifier() {
    let pending_or_ctrl = self.typing_state.caps.is_control()
        || /* need a pending predicate on CapsAsControl */ self.typing_state.caps.is_pending();
    if pending_or_ctrl {
        let mut out = emit_key_events(self, self.typing_state.caps.promote_if_pending());
        let flags = self.typing_state.caps.stamp(ev.key.flags);
        // still apply number/symbol pure remap if desired, or emit raw with CONTROL
        out.push(emit(ev.key.key, ev.key.press, flags));
        return Some(out);
    }
}
None // normal dispatch
```

Add to `CapsAsControl`:

```rust
#[must_use]
pub const fn is_pending(self) -> bool {
    matches!(self.role, Role::Pending)
}
```

Number/symbol invert while caps is control: either the gate calls `laptop::remap` / `symbols::remap` before emit, or those layers never see the key and the gate owns invert for that path. Prefer the gate calling the pure `remap` fns so digits still invert under control.

### Shape B — keep promote in every leaf (rejected)

Does not scale; Home/Nav/every new bind would have to remember. Do not do this.

## Idle path (caps not held)

Unchanged: layer binds and root catch-all as today. Shift dual-role stays in root catch-all / local promote until it moves.

## Delete

`caps_event`, `caps_promote_if_pending`, caps half of `stamp_dual_role_flags`, `CapsRole`.

## Tests

Keep:

- `builtin_caps_down_is_noop`
- `builtin_caps_alone_is_escape`
- `builtin_caps_with_another_key_is_control`
- `builtin_caps_with_digit_is_control_and_inverts`

Add:

```rust
#[test]
fn builtin_caps_plus_home_key_is_control_not_layer() {
    // On Home, caps down then `t` must not enter Typing; must emit ControlLeft down + ctrl+t.
    let mut m = home(); // or boot layer that binds t → typing
    assert_eq!(m.handle(&key(Key::CapsLock)), Some(vec![]));
    let fx = m.handle(&key(Key::KeyT)).expect("effects");
    assert!(fx.iter().any(|e| matches!(e, Emit(KeyEvent { key: Key::ControlLeft, press: Down, .. }))));
    assert!(fx.iter().any(|e| matches!(e, Emit(KeyEvent { key: Key::KeyT, press: Down, flags, .. }) if flags.contains(CONTROL))));
    assert!(matches!(m.layer(), Layer::Home(_))); // still home
}
```

Crate unit tests cover the machine; figaro tests cover gate + held + number path.

---

# Step 3 — `NumberRemaps` is digits only; `SymbolRemaps` gets the rest

Independently shippable; can land before or after steps 1–2. No frebbie change.

## NumberRemaps after

`state/laptop.rs` (or rename file later; name of the struct is `NumberRemaps`):

```rust
//! Built-in number-row shift inversion as a resolve_into layer.
//!
//! Bare `1`..`0` → `!@#$%^&*()` (shift+digit). Shift+digit → bare digit.
//! Brackets and backslash live on [`super::SymbolRemaps`].

#[derive(Bind, Debug)]
#[node(parent = FigaroPath)]
#[binds(FigaroStruct)]
#[bind(
    KeyGroup::Number.down().bare().on_device(DeviceClass::BuiltIn) => invert_number_key,
    KeyGroup::Number.up().bare().on_device(DeviceClass::BuiltIn) => invert_number_key,
    KeyGroup::Number.down().with(ModifierFlags::SHIFT).on_device(DeviceClass::BuiltIn) => invert_number_key,
    KeyGroup::Number.up().with(ModifierFlags::SHIFT).on_device(DeviceClass::BuiltIn) => invert_number_key,
)]
pub struct NumberRemaps {
    #[resolve_into]
    pub next: SymbolRemaps,
}

impl NumberRemaps {
    #[must_use]
    pub(crate) const fn new(next: SymbolRemaps) -> Self {
        Self { next }
    }
}

/// Pure remap: bare digit → same key + SHIFT; shift+digit → same key bare. Else `None`.
#[must_use]
pub fn remap(ev: &KeyEvent) -> Option<KeyEvent> {
    let bare = ev.flags == ModifierFlags::empty();
    let shift_only = ev.flags == ModifierFlags::SHIFT;
    let digit = matches!(
        ev.key,
        Key::Num0
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9
    );
    if !digit {
        return None;
    }
    let out_flags = if bare {
        ModifierFlags::SHIFT
    } else if shift_only {
        ModifierFlags::empty()
    } else {
        return None;
    };
    Some(KeyEvent {
        key: ev.key,
        press: ev.press,
        flags: out_flags,
    })
}
```

Unit tests keep bare/shifted digits; drop backslash and bracket cases from this module.

## SymbolRemaps (new)

`state/symbols.rs`:

```rust
//! Built-in non-digit symbol remaps: brackets ↔ cmd+bracket, backslash ↔ pipe.

use bind::Bind;
use freddie_keys::{Key, KeyEvent, ModifierFlags, WithDevice};

use crate::handlers::*;
use crate::{DeviceClass, FigaroStruct};

use super::{NumberRemapsPath, WisprRemaps};

#[derive(Bind, Debug)]
#[node(parent = NumberRemapsPath)]
#[binds(FigaroStruct)]
#[bind(
    Key::BackSlash.down().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::BackSlash.up().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::BackSlash.down().with(ModifierFlags::SHIFT).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::BackSlash.up().with(ModifierFlags::SHIFT).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::LeftBracket.down().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::LeftBracket.up().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::LeftBracket.down().with(ModifierFlags::COMMAND).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::LeftBracket.up().with(ModifierFlags::COMMAND).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::RightBracket.down().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::RightBracket.up().bare().on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::RightBracket.down().with(ModifierFlags::COMMAND).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
    Key::RightBracket.up().with(ModifierFlags::COMMAND).on_device(DeviceClass::BuiltIn) => invert_symbol_key,
)]
pub struct SymbolRemaps {
    #[resolve_into]
    pub next: WisprRemaps,
}

impl SymbolRemaps {
    #[must_use]
    pub(crate) const fn new(next: WisprRemaps) -> Self {
        Self { next }
    }
}

#[must_use]
pub fn remap(ev: &KeyEvent) -> Option<KeyEvent> {
    let bare = ev.flags == ModifierFlags::empty();
    let shift_only = ev.flags == ModifierFlags::SHIFT;
    let cmd_only = ev.flags == ModifierFlags::COMMAND;
    let (out_key, out_flags) = match (ev.key, bare, shift_only, cmd_only) {
        (Key::BackSlash, true, _, _) => (Key::BackSlash, ModifierFlags::SHIFT),
        (Key::BackSlash, _, true, _) => (Key::BackSlash, ModifierFlags::empty()),
        (Key::LeftBracket | Key::RightBracket, true, _, _) => (ev.key, ModifierFlags::COMMAND),
        (Key::LeftBracket | Key::RightBracket, _, _, true) => (ev.key, ModifierFlags::empty()),
        _ => return None,
    };
    Some(KeyEvent {
        key: out_key,
        press: ev.press,
        flags: out_flags,
    })
}
```

## Handler

`handlers/laptop.rs`: either one generic that takes a remap function, or two thin handlers:

```rust
pub(crate) fn invert_number_key(...) {
    invert_with(ev, st, laptop::remap)
}

pub(crate) fn invert_symbol_key(...) {
    invert_with(ev, st, symbols::remap)
}

fn invert_with<'a, P, F>(
    ev: &DeviceKey,
    st: AscendState<'_, P>,
    remap: F,
) -> (Vec<FigaroEffect>, Completed<P>)
where
    // same bounds as today
    F: FnOnce(&KeyEvent) -> Option<KeyEvent>,
{
    let root: FigaroPath<'a> = st.state.into_ancestor();
    let mut effects = emit_key_events(root, root.typing_state.caps.promote_if_pending());
    // shift_promote as today if still figaro-local
    let physical = &ev.key;
    let remapped = remap(physical).unwrap_or(KeyEvent {
        key: physical.key,
        press: physical.press,
        flags: physical.flags,
    });
    let mut flags = remapped.flags;
    flags = root.typing_state.caps.stamp(flags);
    // shift stamp if Holding*
    effects.push(emit(remapped.key, remapped.press, flags));
    (effects, root.complete())
}
```

## Wiring

`state/mod.rs`:

```rust
pub use laptop::NumberRemaps;
pub use symbols::SymbolRemaps;

pub type NumberRemapsPath<'a> = PathMut<NumberRemaps, FigaroPath<'a>>;
pub type SymbolRemapsPath<'a> = PathMut<SymbolRemaps, NumberRemapsPath<'a>>;
pub type WisprRemapsPath<'a> = PathMut<WisprRemaps, SymbolRemapsPath<'a>>;
// LayerPath parent chain unchanged below Wispr
```

Construction:

```rust
// before
input: NumberRemaps::new(WisprRemaps::new(layer)),
// after
input: NumberRemaps::new(SymbolRemaps::new(WisprRemaps::new(layer))),
```

`wispr.rs` parent path:

```rust
// before
#[node(parent = NumberRemapsPath)]
// after
#[node(parent = SymbolRemapsPath)]
```

## Tests

- Keep digit invert tests (transitions + unit).
- Keep bracket / backslash transition tests if any; unit tests move to `symbols.rs`.
- Caps + digit still promotes control and inverts (step 2 + 3 together).

---

# Order

| Step | What | Repo |
|------|------|------|
| 1 | `freddie_key_remaps` + `CapsAsControl` + unit tests | frebbie |
| 2 | figaro: depend, replace `CapsRole` / free caps fns, keep tests green | figaro |
| 3 | figaro: `NumberRemaps` digits only; add `SymbolRemaps` | figaro |

Step 3 does not depend on 1–2. Steps 1–2 do not depend on 3.

---

# Out of scope (this doc)

- Shift dual-role (`ShiftAlone`) moving into `freddie_key_remaps` (same shape; do after caps lands if wanted).
- Pure number-row invert living in frebbie (figaro policy for now).
- Multiple `#[resolve_into]` siblings (stack stays linear).
- Timer-based tap/hold (not used; alone vs control is only “other key before release”).
