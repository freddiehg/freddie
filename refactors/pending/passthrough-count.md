# passthrough as a count at the root

Not built. "Pass keys through untouched" is one global fact, but it lives in per-layer `AnyKey` catch-alls that re-emit a copy, which drops the flags an injected event carries inline (the Wispr fn-a case). Move it to one count at the root, and make passthrough KEEP the original event instead of re-emitting. Modifier state moves to the root too. Below is the shape as before/after; the ascent/borrow details are left rough on purpose.

## The count and the modifier state on the root

The count is read by the tap thread (synchronously, per key) and written by the model (the guards), so it is an `Arc<AtomicU32>`, not a plain field.

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
    pub passthrough: Arc<AtomicU32>,  // > 0 => pass through; a clone lives on the tap
    #[resolve_into]
    pub power: Power,
}
```

## The guard

New:

```rust
/// Raises the passthrough count while alive, lowers it on drop.
pub struct PassthroughGuard(Arc<AtomicU32>);

impl PassthroughGuard {
    fn new(count: &Arc<AtomicU32>) -> Self {
        count.fetch_add(1, Ordering::Release);
        Self(Arc::clone(count))
    }
}

impl Drop for PassthroughGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}
```

A count, not a bool: pausing while the layer is typing has two guards live, and they drop in an order we do not control. `fetch_add`/`fetch_sub` is order-independent; a bool would clear on the first drop.

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
    _passthrough: PassthroughGuard,           // raises the count while typing is live
}
// SetOfHeldKeys and modify_held_and_pass_through are gone: modifiers are the root's,
// passthrough is the tap's.
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

## Construction goes through a method

The guard needs the root's count, so the variant is built by a method that can reach it, never a struct literal.

Before (`to_typing`):

```rust
*node.parent.ascend().get_mut() = Layer::Typing(TypingLayer::default());
```

After:

```rust
let count = node.parent.ascend_to::<MercuryPath>().passthrough.clone(); // roughly
// ... then set the Layer via the ascent, built through the method:
Layer::typing(&count)
```

```rust
impl Layer {
    fn typing(count: &Arc<AtomicU32>) -> Self {
        Self::Typing(TypingLayer { _passthrough: PassthroughGuard::new(count) })
    }
}
```

The increment can't be forgotten (it is in the constructor), and leaving the layer drops the `TypingLayer`, whose guard decrements. Same for `Power::pause` building `Paused` through a method.

## The tap keeps the original instead of re-emitting

Before (`main.rs`):

```rust
move |ev| {
    let _ = event_tx.send(MercuryEvent::Key(ev));
    None // always drop; the effect loop re-emits a copy, losing inline flags
}
```

After:

```rust
move |ev| {
    let _ = event_tx.send(MercuryEvent::Key(ev.clone())); // model still sees it (commands, state)
    if passthrough.load(Ordering::Acquire) > 0 {
        Some(ev) // Some(same) => decide() => Pass => tap KEEPS the original, flags intact
    } else {
        None     // command mode: drop, the async model handles it
    }
}
```

`passthrough` is a clone of the root's `Arc<AtomicU32>`, read on the tap thread. Keeping the original is what carries Wispr's inline `cmd` flag through; the re-emit dropped it.

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

- `Arc<AtomicU32>` in the state tree versus explicit increment/decrement in the transition methods without `Drop` (keeps the tree pure values, loses "can't forget").
- The command-key exception in the tap: how it knows synchronously which keys the current passthrough state still owns.
- Held modifiers at the root under one-handler-per-event, and whether this forces the `no-clobber.md` decision; the `cmd`-`alt`-`p` unpause and typing's escape also move to the root.
- `u8` versus `u32` for the count (two sources today; it does not matter).
