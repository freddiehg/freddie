# passthrough as a count at the root

Not built. "Pass keys through untouched" is one global fact, but it lives in per-layer `AnyKey` catch-alls that re-emit a copy, which drops the flags an injected event carries inline (the Wispr fn-a case). Move it to one count at the root, and make passthrough KEEP the original event instead of re-emitting. Modifier state moves to the root too. Below is the shape, before/after.

## The count on the root

A small newtype, `PassthroughCount`, over a shared `Rc<Cell<u8>>`. Single-threaded (dispatch is synchronous, the tap runs the model inline; `synchronous-dispatch.md`), so `Rc`/`Cell`, not `Arc`/atomic. It is shared rather than a plain field so the guard that raises it can also lower it from `Drop` (below): a guard `Drop` gets only `&mut self`, so it must hold the count, not reach for it.

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
    pub held: HeldModifiers,          // one copy of what's held, here, not per-layer
    pub passthrough: PassthroughCount, // > 0 => pass through
    #[resolve_into]
    pub power: Power,
}

/// The passthrough count. `> 0` means keep keys as-is.
///
/// Wrapped so nothing pokes the raw integer, and SHARED (`Rc<Cell>`) so a guard's `Drop` can
/// decrement it. `Drop::drop(&mut self)` gets only `&mut self`, never `&mut Mercury`, so a guard
/// can't reach a plain count on the root from `Drop` -- it has to CLOSE OVER the count. An
/// `Rc<Cell>` is exactly that: a handle to the one count the guard holds a clone of and mutates
/// through, from `Drop`, with nothing else passed in.
///
/// Single-threaded (dispatch is synchronous), so `Rc<Cell>`, not `Arc<Atomic>`. `Cell`, not
/// `RefCell`: the count is a `Copy` `u8` we only get and set wholesale, so there is nothing to
/// borrow -- `RefCell`'s runtime borrow-checking and `already borrowed` panic buy nothing here.
/// `u8`: at most two sources (typing, paused).
#[derive(Clone, Default)]
pub struct PassthroughCount(Rc<Cell<u8>>);

impl PassthroughCount {
    #[must_use]
    pub fn passing(&self) -> bool {
        self.0.get() > 0
    }
    // crate-private, only for the guard. `&self` (not `&mut`): the Cell is the mutability.
    fn increment(&self) {
        self.0.set(self.0.get() + 1);
    }
    fn decrement(&self) {
        self.0.set(self.0.get() - 1);
    }
}
```

## The guard closes over the count and decrements on drop

`Drop` gets only `&mut self`, so for `Drop` to do the decrement the guard has to HOLD the count, not reach for it. It holds a clone of the `Rc<Cell>` and decrements through it on drop. That is the whole reason for `Rc` over a plain `u8` on the root: it lets an ordinary `Drop` do the right thing.

```rust
pub struct PassthroughGuard(PassthroughCount); // a clone of the shared count

impl PassthroughGuard {
    fn new(count: &PassthroughCount) -> Self {
        count.increment();
        Self(count.clone()) // clone the Rc, so Drop can reach the count
    }
}

impl Drop for PassthroughGuard {
    fn drop(&mut self) {
        self.0.decrement();
    }
}
```

Ordinary `Drop`, so nothing infects: `PassthroughGuard` is droppable, therefore `TypingLayer`, `Layer`, `Power`, and `Mercury` all stay droppable, and dropping any of them just runs the decrement. No `prevent_drop` link error, no panic-on-drop, no explicit release.

A count, not a bool: pausing while the layer is typing has two guards live, and they drop in an order we do not control. Increment/decrement is order-independent; a bool would clear on the first drop while the other source still wants passthrough.

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
    _passthrough: PassthroughGuard,           // RAII: decrements the count when dropped
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
    _passthrough: PassthroughGuard,
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
let guard = PassthroughGuard::new(&root.passthrough);        // &root: clone the count, increment
*root.power.layer_mut() = Layer::Typing(TypingLayer { _passthrough: guard }); // &mut root: set
```

The `&root.passthrough` borrow ends when `new` returns (the guard owns a `clone`, not a borrow), so the `&mut root` assignment that follows is fine. Same shape for `Power::pause` building `Paused`.

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
    if root.passthrough.passing() {
        Some(ev) // Some(same) => decide() => Pass => tap KEEPS the original, flags intact
    } else {
        None     // command mode: drop
    }
}
```

`root.passthrough` is a plain read on the same thread; no atomic, no channel. Keeping the original is what carries Wispr's inline `cmd` flag through; the re-emit dropped it.

Exception: some keys in passthrough are still commands (typing's escape, the `cmd`-`alt`-`p` unpause). Those must NOT be kept — the tap has to drop them so the model acts. So the rule is "keep unless the current passthrough state still claims this key as a command," and how the tap knows that synchronously is the open part.

## AnyKey stops matching modifiers

Before (`sources.rs`):

```rust
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, _ev: &KeyEvent) -> bool {
        true
    }
}
```

After:

```rust
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        !ev.key.is_modifier() // modifiers are the root's business, never a layer catch-all's
    }
}
```

(`Key::is_modifier` is a small helper to add in `freddie_keys`.)

## Open questions

- The command-key exception in the tap: how it knows synchronously which keys the current passthrough state still owns (typing's escape, the `cmd`-`alt`-`p` unpause), so it drops those rather than keeping them.
- The release-on-leave borrow: a transition needs `&mut root` (to decrement) and the old layer (to pull the guard out) at once. Working that out against the path types is the fiddly part.
- Held modifiers at the root under one-handler-per-event, and whether this forces the `no-clobber.md` decision; the `cmd`-`alt`-`p` unpause and typing's escape also move to the root.
