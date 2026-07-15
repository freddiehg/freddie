# passthrough and held modifiers at the root

Depends on `remove-pause.md`: with pause gone there is exactly ONE passthrough layer (typing), so "are we passing through" is a bool read off the tree, not a count maintained by guards. This doc folds in the old `root-passthrough.md` (now deleted): the root owns the passthrough decision for EVERY key, and the layers keep only their commands.

The shape:

- The root owns `HeldModifiers`, the physical truth about which modifier keys are down. `handle` updates it on every key, in every state, unconditionally, so no handler maintains it.
- The root passes a key through when the active layer is a passthrough layer and no layer bound the key. A layer that binds the key CLOBBERS the passthrough: its command runs and the key is not re-emitted.
- Entering or leaving a passthrough layer flushes the app's modifier view, and the flush is computed in ONE place, `handle`, by diffing the passthrough predicate across dispatch. No transition handler emits it.

That last point is the reason the guard/count design in the previous draft is gone. There, every handler that could leave a passthrough layer had to remember to emit the close, and every handler that entered one had to emit the open. Leaving typing for nav, for resize, for home, for in-app, on quit: each drop site was a place to get the flush wrong. Diffing the predicate in `handle` collapses all of them into the one function that already sees every event.

## The passthrough predicate lives on the layer (`state.rs`)

"Is the active layer a passthrough layer" is a method on `Layer`, the single source of the passthrough decision:

```rust
impl Layer {
    /// A passthrough layer re-emits every key the active layer did not bind. Typing is the only
    /// one today; more can be added by returning true for them.
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        matches!(self, Layer::Typing(_))
    }
}
```

There is no `Passthrough` struct, no `active` count, no `Rc<Cell>`, no `PassthroughGuard`. Those existed only to count NESTED passthrough states (paused-over-typing), which `remove-pause.md` deletes. With one passthrough layer, the tree already holds the answer.

## `handle` tracks, flushes at the boundary, and passes through (`state.rs`)

`handle` is the whole design. It:

1. Tracks the key into `held`, always, whatever the layer, so `held` is the physical truth.
2. Snapshots `is_passthrough` before and after dispatch, and emits the flush on a boundary crossing.
3. Passes the key through when dispatch declined it (`None`) and the active layer is a passthrough layer.

`bind::dispatch` returns `None` exactly when nothing on the active path bound the key, and `Some(effects)` when a handler ran (even if it returned no effects). So `None` is precisely "no layer claimed this key," which is the fall-through case the root handles. A bound key returns `Some` and clobbers the passthrough.

Before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

After:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let key = if let MercuryEvent::Key(ev) = event { Some(ev) } else { None };
    if let Some(ev) = key {
        self.held.track(ev); // physical truth, whatever the layer
    }

    let was_passthrough = self.layer().is_passthrough();
    let dispatched = bind::dispatch::<MercuryStruct, Self>(self, event);
    let now_passthrough = self.layer().is_passthrough();

    let mut effects = Vec::new();

    // Enter (false -> true): open, so the app catches up on modifiers held before we entered.
    // Prepended, so the catch-up lands before whatever the entering command emitted.
    if !was_passthrough && now_passthrough {
        effects.extend(self.held.open());
    }

    match dispatched {
        // A layer bound the key: its command runs, and it clobbers the passthrough.
        Some(layer_effects) => effects.extend(layer_effects),
        // No layer bound the key: the root passes it through iff we are in a passthrough layer.
        None => {
            if let Some(ev) = key {
                if now_passthrough {
                    effects.push(emit(ev.key, ev.press, self.held.flags()));
                }
            }
        }
    }

    // Leave (true -> false): close, so the modifiers held on the way out are released. The
    // command layer would swallow their real ups; without this they stay stuck. Appended, so
    // the release lands after whatever the leaving command emitted.
    if was_passthrough && !now_passthrough {
        effects.extend(self.held.close());
    }

    Some(effects)
}
```

In every real transition the key that crosses the boundary is a bound command (`t` enters typing, `cmd`-`escape` leaves it), so `dispatched` is `Some` with no effects and the ordering of flush-versus-command is not observable. The prepend/append is for readability, not correctness.

There is no `AnyNonModifierKey` and no modifier special-case in `handle`. A modifier key is an ordinary key: no layer binds it, so dispatch returns `None`, and the root passes it through iff we are in a passthrough layer, exactly like every other unbound key. `held.track` is what makes a modifier special, and only in that it updates `held`; the emit-versus-swallow decision is the same one every key gets. This is `modifier-keys.md`'s "modifiers are not special" made real.

## The root gains `held`, loses the guard plumbing (`state.rs`)

Before (after `remove-pause.md`):

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    #[resolve_into]
    pub layer: Layer,
}
```

After:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    pub held: HeldModifiers,
    #[resolve_into]
    pub layer: Layer,
}
```

`Default` adds `held: HeldModifiers::default()`. `held` is a plain field the root reads and `handle` writes; nothing hands out a handle to it.

## Held modifiers (`state.rs`)

`held` on the root is the physical truth about the modifier keys, updated by `track` and read by `flags`/`open`/`close`.

```rust
#[derive(Debug, Default, Clone, Copy)]
pub struct LeftRightPair {
    pub left: bool,
    pub right: bool,
}

pub enum Side { Left, Right }

impl LeftRightPair {
    #[must_use]
    pub const fn any_held(&self) -> bool { self.left || self.right }

    pub const fn set(&mut self, side: Side, is_down: bool) {
        match side {
            Side::Left  => self.left = is_down,
            Side::Right => self.right = is_down,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HeldModifiers {
    pub control: LeftRightPair,
    pub meta: LeftRightPair,
    pub alt: LeftRightPair,
    pub shift: LeftRightPair,
    pub caps_lock: bool,
}

impl HeldModifiers {
    pub fn track(&mut self, ev: &KeyEvent) {
        let is_down = ev.press == PressType::Down;
        match ev.key {
            Key::ControlLeft  => self.control.set(Side::Left,  is_down),
            Key::ControlRight => self.control.set(Side::Right, is_down),
            Key::MetaLeft     => self.meta.set(Side::Left,     is_down),
            Key::MetaRight    => self.meta.set(Side::Right,    is_down),
            Key::AltLeft      => self.alt.set(Side::Left,      is_down),
            Key::AltRight     => self.alt.set(Side::Right,     is_down),
            Key::ShiftLeft    => self.shift.set(Side::Left,    is_down),
            Key::ShiftRight   => self.shift.set(Side::Right,   is_down),
            Key::CapsLock     => self.caps_lock = is_down,
            _ => {}
        }
    }

    /// Entering emits a DOWN for every held key; leaving emits an UP. Read-only. Each event carries
    /// the flags as they stand after its own change (`shown` is what has been emitted so far), so a
    /// shared left/right bit clears only when both sides are up.
    pub fn open(&self)  -> Vec<MercuryEffect> { self.sweep(PressType::Down) }
    pub fn close(&self) -> Vec<MercuryEffect> { self.sweep(PressType::Up) }

    fn sweep(&self, press: PressType) -> Vec<MercuryEffect> {
        let mut shown = if press == PressType::Down { Self::default() } else { *self };
        let mut out = Vec::new();
        for key in Self::MODIFIER_KEYS {
            if self.is_down(key) {
                shown.track(&KeyEvent { key, press, flags: ModifierFlags::empty() }); // track ignores flags
                out.push(emit(key, press, shown.flags()));
            }
        }
        out
    }

    const MODIFIER_KEYS: [Key; 9] = [
        Key::ControlLeft, Key::ControlRight, Key::MetaLeft, Key::MetaRight,
        Key::AltLeft, Key::AltRight, Key::ShiftLeft, Key::ShiftRight, Key::CapsLock,
    ];

    fn is_down(&self, key: Key) -> bool {
        match key {
            Key::ControlLeft  => self.control.left,
            Key::ControlRight => self.control.right,
            Key::MetaLeft     => self.meta.left,
            Key::MetaRight    => self.meta.right,
            Key::AltLeft      => self.alt.left,
            Key::AltRight     => self.alt.right,
            Key::ShiftLeft    => self.shift.left,
            Key::ShiftRight   => self.shift.right,
            Key::CapsLock     => self.caps_lock,
            _ => false,
        }
    }

    #[must_use]
    pub fn flags(&self) -> ModifierFlags {
        let mut f = ModifierFlags::empty();
        f.set(ModifierFlags::CONTROL,   self.control.any_held());
        f.set(ModifierFlags::COMMAND,   self.meta.any_held());
        f.set(ModifierFlags::ALT,       self.alt.any_held());
        f.set(ModifierFlags::SHIFT,     self.shift.any_held());
        f.set(ModifierFlags::CAPS_LOCK, self.caps_lock);
        f
    }
}

fn emit(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}
```

`open`/`close` are `pub` now: `handle` calls them directly on the boundary crossing, there is no guard between them and the caller. `sweep` needs `HeldModifiers: Copy` for the leaving snapshot (all-bool; `Clone, Copy` on it and `LeftRightPair`). `fn` is not a `Key` variant, so it is punted; `caps_lock` is a plain bool, and whether a toggle belongs in an open/close sweep is an open question below.

## Held modifiers stay on the ROOT, not on typing

With one passthrough layer it is tempting to move `held` onto `TypingLayer` and delete it from the root. That does not work: the flush needs `held` to persist ACROSS the entry and exit, and to be readable at the boundary in `handle`.

- On entry, `open` reads what is held BEFORE the passthrough layer exists, to catch the app up. A `held` constructed with the typing layer starts empty, so there is nothing to open with. The modifiers were pressed while some earlier layer was live, and the root is the only node that outlives that transition.
- On exit, `close` reads what is held as typing is torn down. A `held` owned by the typing layer is dropped with the layer, so there is nothing left to close with, and the modifiers stay stuck.

`held` has to live above the layer that comes and goes, which is the root. The layer being a passthrough layer is a fact about its TYPE (`is_passthrough`); the modifiers held while it is active are a fact about the KEYBOARD, and those two facts have different lifetimes.

## The layers become markers (`state.rs`)

Every layer loses its catch-all. A layer keeps only its real commands, and the root passes the rest through (or swallows them, in a command layer). Typing loses `AnyKey`, its `SetOfHeldKeys`, and the `HeldModifiers` it used to carry:

Before:

```rust
pub struct SetOfHeldKeys { pub cmd: Option<Key> }

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
#[bind(Key::Escape.down() => maybe_go_home)]
pub struct TypingLayer {}
```

Typing binds one thing, `escape`, and only because `cmd`-`escape` is a real command (leave to home) that has to clobber the root passthrough. Everything else in typing (letters, digits, modifiers, plain `escape`) is unbound, returns `None` from dispatch, and the root passes it through.

`AnyKey` stays as the trigger the paused arm used, and dies with `remove-pause.md`; no layer binds a catch-all after this. There is no `AnyNonModifierKey`: the whole point is that the root handles the fall-through for every key uniformly, so there is no trigger to write.

## Transition handlers become dumb (`home.rs`, `typing.rs`)

No handler emits a flush any more. `to_typing` just sets the layer; `handle` sees `is_passthrough` go true and emits the open:

```rust
pub(crate) fn to_typing<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    *node.parent.ascend().get_mut() = Layer::Typing(TypingLayer {});
    Vec::new()
}
```

`maybe_go_home` decides exit-versus-type by reading the root's `held`, and on exit just sets the layer and swallows the escape. It does NOT emit the `cmd` release: `handle` sees `is_passthrough` go false and emits the close, which releases every held modifier, not just `cmd`.

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.held.meta.any_held() {
        go_home(&mut node.parent.into_parent()); // set Layer::Home; escape swallowed
        Vec::new()
    } else {
        vec![emit(ev.key, ev.press, root.held.flags())] // plain escape passes through, with flags
    }
}
```

The non-exit branch emits the escape itself, because binding `escape` clobbered the root passthrough: a bound key does not fall through, so if typing wants a plain `escape` to pass through it has to re-emit it. That is the one key typing re-emits by hand, and only because it also has a command on it.

## Quit and drop do nothing

There are no guards to drop, so quit and tree teardown produce no flush, which is correct: a close is only needed because a LIVE mercury swallows the real key-up in a command layer. On quit or drop the `Interceptor` releases the grab, so the keys still physically held deliver their real ups to the OS and app. A synthetic close here would fire an up while the key is still physically down, ahead of the real one. `on_quit` returns `vec![MercuryEffect::Kill]` unchanged.

## Setting the flags on emitted keys

Why flags at all: a `CGEvent`'s modifier flags are baked in when it is created, from the event source's state, and that state only catches up once a posted modifier has been PROCESSED. So a chord posted back to back, `cmd` down then `r`, creates the `r` microseconds later carrying no `cmd`. The output has to state its own flags rather than trust the source.

The flags live ON `KeyEvent`, so every event carries exactly its own, with no accumulator anyone shares:

```rust
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags, // new
}
```

This is what kills clobbering. The emitter's old `self.flags`/`next_flags` was a running accumulator, so a `cmd`-`r` synthesized in in-app mode WHILE the user holds `shift` came out `cmd`-`shift`-`r`, the chord inheriting an unrelated held modifier. With the flags on the event, a `cmd`-`r` carries `{cmd}` and nothing else whatever is held; a passthrough `v` carries `{cmd}` because `cmd` IS held; they never bleed.

Whoever builds the event supplies its flags:

- Passing a key through (the fall-through emit in `handle`, the plain-escape branch of `maybe_go_home`): `held.flags()`. `track` has run, so a modifier's own event is right (`cmd`-down carries `cmd`, `cmd`-up does not) and a non-modifier carries what is held.
- A flush (`open`/`close`): `sweep` stamps `shown.flags()` per event, so a CLOSE's `cmd`-up carries `cmd` CLEAR while it is still physically held, and a shared left/right bit stays set until its second side is up.
- A synthesized chord (`refresh`'s `cmd`-`r`): its OWN flags, from the chord, independent of `held`.

The emitter is then dumb, applying the event's flags. `self.flags`, `next_flags`, `flags_for`, and its `Cell` are deleted. Before/after (`macos.rs`):

```rust
// before: a shared running accumulator (the clobber)
fn emit(&self, key: Key, down: bool) {
    self.flags.set(next_flags(self.flags.get(), key, down));
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | self.flags.get());
}

// after: apply exactly this event's flags
fn emit(&self, ev: &KeyEvent) {
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(ev.flags));
}
```

`ModifierFlags` is a portable bitset in `freddie_keys` (`CONTROL`/`COMMAND`/`ALT`/`SHIFT`/`CAPS_LOCK`); `to_cg` maps it to `CGEventFlags`, built from the existing per-key `flag_of`.

## Open questions

- `caps_lock` is a physical toggle, not a held key. Whether it belongs in the open/close sweep at all, or should be excluded from `MODIFIER_KEYS`, is unresolved; the sweep treats it like the rest for now.
- `fn` is not a `Key` variant, so `held` cannot track it and a `fn`-modified passthrough is not caught up. Punted with the Wispr fn-a fix (keeping the ORIGINAL event so injected inline flags survive), a separate follow-up.
- The `IfActivePassthru` structural form from the old `root-passthrough.md`: a child of the root, present iff a passthrough layer is active, holding the catch-all, so the passthrough decision is whether the child resolves rather than a branch in `handle`. It needs laserbeam multiple-children (the `no-clobber.md` multi-cast decision) and the `Option`/fallible resolve from `laserbeam-state-controlled-children.md`. Until then `handle` branches on `is_passthrough`, which is the whole of this doc. Not blocking anything.
