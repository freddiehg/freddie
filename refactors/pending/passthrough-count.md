# passthrough as a count at the root

Not built. "Pass keys through untouched" is one global fact, but it lives in per-layer `AnyKey` catch-alls that re-emit a copy, which drops the flags an injected event carries inline (the Wispr fn-a case). Move it to one count at the root, and make passthrough KEEP the original event instead of re-emitting. Modifier state moves to the root too. Below is the shape as before/after; the ascent/borrow details are left rough on purpose.

## The count on the root

A plain `u32`. Single-threaded (dispatch is synchronous, the tap runs the model inline; `synchronous-dispatch.md`), and the handlers that enter and leave typing/paused already hold `&mut Mercury` — they ascend to the root — so they bump it directly. No `Cell`, no `Rc`, no atomic.

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
    pub held: HeldModifiers,  // one copy of what's held, here, not per-layer
    pub passthrough: u32,     // > 0 => pass through
    #[resolve_into]
    pub power: Power,
}
```

## The guard is a linear token, released explicitly (`prevent_drop`)

`Drop::drop(&mut self)` gets no `&mut Mercury`, and the count lives on the root, so a guard can't decrement on plain `Drop`. So it is CONSUMED by a `release(self, &mut Mercury)` that decrements, and dropping it instead of releasing is forbidden.

Use the `prevent_drop` crate for that, which is built for exactly this linear-resource case:

- Its generated `Drop` calls an unresolved `extern` symbol, so any drop the linker can SEE is a link error. Leaving the guard un-released fails to build, not at run time.
- For the paths the linker can't see (a panic unwinding between acquire and release), it falls back to a runtime panic or abort.
- Its cleanup function may take extra parameters, so `release` takes `&mut Mercury` and decrements. That is the whole reason we can't use ordinary `Drop`.

So, in shape (exact macro API is `prevent_drop`'s):

```rust
pub struct PassthroughGuard { /* prevent_drop token, no fields of ours; the count is on the root */ }

impl PassthroughGuard {
    fn acquire(root: &mut Mercury) -> Self {
        root.passthrough += 1;
        // ... construct the prevent_drop token ...
    }

    // the custom cleanup, which prevent_drop lets take parameters:
    fn release(self, root: &mut Mercury) {
        root.passthrough -= 1;
        // ... defuse the token ...
    }
}
// a reachable drop is a LINK error; a drop only on a panic-unwind path is a runtime panic/abort;
// neither is a silent leak of the count.
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
    passthrough: PassthroughGuard,            // held while typing is live; released on leave
}
// SetOfHeldKeys and modify_held_and_pass_through are gone: modifiers are the root's,
// passthrough is the tap's. The guard is a MANAGED field, not `_passthrough`: leaving
// typing must call .release(root), or its Drop panics.
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
    passthrough: PassthroughGuard,
}
// no AnyKey bind (passthrough is the tap); the cmd-alt-p unpause moves to the root.
```

## Entering acquires, leaving releases

The handler that enters typing/paused holds `&mut Mercury` (it ascends to the root), so it acquires the guard there; the variant is built through a method, never a struct literal, so the acquire cannot be skipped.

Enter, before (`to_typing`):

```rust
*node.parent.ascend().get_mut() = Layer::Typing(TypingLayer::default());
```

Enter, after:

```rust
let root = node.parent.ascend_to::<MercuryPath>();
// build through the method, which acquires the guard against the root
root.set_layer(Layer::typing(root)); // roughly; the borrow of root is the fiddly part
```

```rust
impl Layer {
    fn typing(root: &mut Mercury) -> Self {
        Self::Typing(TypingLayer { passthrough: PassthroughGuard::acquire(root) })
}
```

Leaving is the mirror, and it is the fiddly part: a transition away from typing/paused cannot just reassign the layer, because dropping the old `TypingLayer` would drop its guard and panic. The transition has to pull the guard out of the old layer and `release(root)` it first, then set the new layer. So `go_home` and friends grow a "if we're leaving a passthrough layer, release its guard against the root" step. That step is exactly the balance the panic-on-drop is enforcing: forget it and you get a crash, not a stuck count. Same for `Power::unpause` releasing `Paused`'s guard.

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
    if root.passthrough > 0 {
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

- `Arc<AtomicU32>` in the state tree versus explicit increment/decrement in the transition methods without `Drop` (keeps the tree pure values, loses "can't forget").
- The command-key exception in the tap: how it knows synchronously which keys the current passthrough state still owns.
- Held modifiers at the root under one-handler-per-event, and whether this forces the `no-clobber.md` decision; the `cmd`-`alt`-`p` unpause and typing's escape also move to the root.
- `u8` versus `u32` for the count (two sources today; it does not matter).
