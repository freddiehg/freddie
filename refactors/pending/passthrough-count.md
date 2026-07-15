# passthrough as a count at the root

Not built. "Pass keys through untouched" is one global fact, but it lives in per-layer `AnyKey` catch-alls that re-emit a copy, which drops the flags an injected event carries inline (the Wispr fn-a case). Move it to one count at the root, and make passthrough KEEP the original event instead of re-emitting. Modifier state moves to the root too. Below is the shape, before/after.

## The count on the root

A small newtype, `ActivePassthroughLayer`, over a shared `Rc<Cell<u8>>`, exposed as a drop guard (`is_active()` to ask, `guard()` to raise). Single-threaded (dispatch is synchronous, the tap runs the model inline; `synchronous-dispatch.md`), so `Rc`/`Cell`, not `Arc`/atomic. It is shared rather than a plain field so the guard that raises it can also lower it from `Drop` (below): a guard `Drop` gets only `&mut self`, so it must hold the count, not reach for it.

Before:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    #[resolve_into]
    pub power: Power,
}
```

After:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    pub held: HeldModifiers,                 // one copy of what's held, here, not per-layer
    pub passthrough: ActivePassthroughLayer, // .is_active() => modifiers are emitted
    #[resolve_into]
    pub power: Power,
}

/// Whether modifiers (and keys) currently pass through. `is_active()` is the question everyone
/// asks; `guard()` is the only way to raise it, and the returned guard lowers it on drop.
///
/// Internally a count, not a bool (`u8` behind `Rc<Cell>`), because overlapping sources (typing,
/// paused) raise it and drop in an order we don't control; a bool would clear on the first drop
/// while the other still wants passthrough. That's an impl detail: the API is the drop guard, not
/// the counter.
///
/// SHARED (`Rc<Cell>`) so the guard's `Drop` can lower the count. `Drop::drop(&mut self)` gets
/// only `&mut self`, never `&mut Mercury`, so the guard has to CLOSE OVER the count rather than
/// reach for it; an `Rc<Cell>` clone is exactly that handle. Single-threaded (dispatch is
/// synchronous), so `Rc`/`Cell`, not `Arc`/atomic. `Cell`, not `RefCell`: the `u8` is `Copy` and
/// swapped wholesale, so there's nothing to borrow and `RefCell`'s runtime checks/panic buy
/// nothing.
#[derive(Clone, Default)]
pub struct ActivePassthroughLayer(Rc<Cell<u8>>);

impl ActivePassthroughLayer {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.0.get() > 0
    }

    /// Raise the flag until the returned guard drops. The only way to raise it, so every raise is
    /// balanced by a `Drop`.
    #[must_use]
    pub fn guard(&self) -> PassthroughLayerGuard {
        PassthroughLayerGuard::new(self)
    }

    // internal, the same u8 inc/dec as ever; not the public API. `&self`: the Cell is the mutability.
    fn increment(&self) {
        self.0.set(self.0.get() + 1);
    }
    fn decrement(&self) {
        self.0.set(self.0.get() - 1);
    }
}
```

## The guard lowers the flag on drop

```rust
/// Holds the flag up while alive, lowers it on drop. Stored on `TypingLayer` and `Paused`.
pub struct PassthroughLayerGuard(ActivePassthroughLayer); // a clone of the shared flag

impl PassthroughLayerGuard {
    fn new(flag: &ActivePassthroughLayer) -> Self {
        let guard = Self(flag.clone());
        guard.0.increment();
        guard
    }
}

impl Drop for PassthroughLayerGuard {
    fn drop(&mut self) {
        self.0.decrement();
    }
}
```

Ordinary `Drop`, so nothing infects: `PassthroughLayerGuard` is droppable, therefore `TypingLayer`, `Layer`, `Power`, and `Mercury` all stay droppable, and dropping any of them just runs the decrement. No `prevent_drop` link error, no panic-on-drop, no explicit release. The guard closing over the flag is what lets `Drop` (which gets only `&mut self`) do the decrement at all.

## Typing and Paused hold a guard

Before:

```rust
#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyKey => modify_held_and_pass_through,
)]
pub struct TypingLayer {
    pub held: SetOfHeldKeys,
}
```

After:

```rust
#[bind(Key::Escape.down() => maybe_go_home)]  // only its command stays
pub struct TypingLayer {
    _passthrough: PassthroughLayerGuard,           // RAII: decrements the count when dropped
}
// SetOfHeldKeys and modify_held_and_pass_through are gone: modifiers are the root's,
// passthrough is the tap's. `_passthrough` is held only for its Drop; nothing reads it.
```

Before:

```rust
#[bind(AnyKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    pub held: HeldModifiers,
}
```

After:

```rust
pub struct Paused {
    pub layer: Layer,
    _passthrough: PassthroughLayerGuard,
}
// no AnyKey bind (passthrough is the tap); the cmd-alt-p unpause moves to the root.
```

## Entering builds the guard; leaving is a normal layer set

Because the guard decrements on `Drop`, leaving is just a normal assignment: reassigning the layer drops the old `TypingLayer`, which drops its guard, which decrements. `go_home` and `unpause` stay plain `*layer = ...`; no release, no special-casing.

Only entering does anything new: build the guard from the root's count. It needs the count (to clone the `Rc`) and then to set the layer, which is two sequential borrows of the root, not the one-expression double-borrow that would not compile.

Enter, before (`to_typing`):

```rust
*node.parent.ascend().get_mut() = Layer::Typing(TypingLayer::default());
```

Enter, after:

```rust
let root = node.parent.ascend_to::<MercuryPath>();
let guard = root.passthrough.guard();                        // &root: raise the flag, get a guard
*root.power.layer_mut() = Layer::Typing(TypingLayer { _passthrough: guard }); // &mut root: set
```

The `&root.passthrough` borrow ends when `guard()` returns (the guard owns a `clone`, not a borrow), so the `&mut root` assignment that follows is fine. Same shape for `Power::pause` building `Paused`.

## The tap keeps the original instead of re-emitting

Before (`main.rs`):

```rust
move |ev| {
    let _ = event_tx.send(MercuryEvent::Key(ev));
    None // always drop; the effect loop re-emits a copy, losing inline flags
}
```

After (synchronous dispatch, so `on_key` runs the model inline and holds the root):

```rust
move |ev| {
    dispatch(&mut root, &MercuryEvent::Key(ev.clone())); // commands, state, guard inc/dec
    if root.passthrough.is_active() {
        Some(ev) // Some(same) => decide() => Pass => tap KEEPS the original, flags intact
    } else {
        None     // command mode: drop
    }
}
```

`root.passthrough` is a plain read on the same thread; no atomic, no channel. Keeping the original is what carries Wispr's inline `cmd` flag through; the re-emit dropped it.

Exception: some keys in passthrough are still commands (typing's escape, the `cmd`-`alt`-`p` unpause). Those must NOT be kept — the tap has to drop them so the model acts. So the rule is "keep unless the current passthrough state still claims this key as a command," and how the tap knows that synchronously is the open part.

## Held modifiers

`held` is a plain struct with a bool per modifier key, all of them, left and right distinguished, so the model always knows exactly what is down. No `Option<Key>`, no "which cmd" ambiguity.

```rust
#[derive(Debug, Default)]
pub struct HeldModifiers {
    pub meta_left: bool,
    pub meta_right: bool,
    pub control_left: bool,
    pub control_right: bool,
    pub alt_left: bool,
    pub alt_right: bool,
    pub shift_left: bool,
    pub shift_right: bool,
}
```

Modifiers stay ordinary keys: a catch-all handler still matches every key and just branches, `matches!(ev.key, Key::MetaLeft)` and friends, setting the matching bool on each down/up. Making the catch-all trigger itself skip modifiers (a `NonModifierKey`) is a follow-up, not this doc -- `modifier-keys.md`'s point is that modifiers are not special, so nothing at the trigger level should treat them so.

A `u128` bitset over every key, one bit each, is the eventual tighter form (set/test with bit magic), but the explicit bool struct is fine for now.

## Open questions

- The command-key exception in the tap: how it knows synchronously which keys the current passthrough state still owns (typing's escape, the `cmd`-`alt`-`p` unpause), so it drops those rather than keeping them.
- The release-on-leave borrow: a transition needs `&mut root` (to decrement) and the old layer (to pull the guard out) at once. Working that out against the path types is the fiddly part.
- Where `held` lives (root vs the layer that needs it) under one-handler-per-event, and whether that forces the `no-clobber.md` decision.
- Deferred follow-ups: the `NonModifierKey` trigger, the `u128` held-keys bitset, and reconciling the emitter's own modifier-flag reconstruction (`self.flags` / `next_flags`) with `held` so modifier state has a single home rather than one tracker in the model and another in the emitter.
