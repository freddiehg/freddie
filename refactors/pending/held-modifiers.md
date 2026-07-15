# passthrough and held modifiers at the root

Builds on `remove-pause.md`: typing is the only passthrough layer.

The root owns `HeldModifiers`, the physical truth about which modifier keys are down. Two catch-all handlers bound on the root do the passthrough: a modifier key updates `held` and passes through while the active layer is a passthrough layer; every other key passes through while a passthrough layer is active, and is swallowed otherwise. Entering or leaving a passthrough layer flushes the app's modifier view, and that flush belongs to `set_layer`, the one method every layer transition goes through.

`Layer::is_passthrough` is the single passthrough test:

```rust
impl Layer {
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        matches!(self, Layer::Typing(_))
    }
}
```

## `Mercury` (`state.rs`)

```rust
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_passthru,
)]
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    pub held: HeldModifiers,
    #[resolve_into]
    layer: Layer,
}
```

`held` is read by handlers (typing checks `root.held.meta`); `layer` is private, mutated only through `set_layer`.

## `HeldModifiers` (`state.rs`)

`track` records each modifier key's up and down. `flags` reads the current state as a bitset. `open` emits a DOWN for every held key, `close` an UP; each swept event carries the flags as they stand after its own change, so a shared left/right bit clears only when both sides are up. `caps_lock` is a lock, not a held key: it changes on press, so it cannot be swept and is not tracked here. It is not a modifier, so `AnyNonModifierKey` matches it and `maybe_passthru` passes it through like any other key.

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
            _ => {}
        }
    }

    pub fn open(&self)  -> Vec<MercuryEffect> { self.sweep(PressType::Down) }
    pub fn close(&self) -> Vec<MercuryEffect> { self.sweep(PressType::Up) }

    fn sweep(&self, press: PressType) -> Vec<MercuryEffect> {
        let mut shown = if press == PressType::Down { Self::default() } else { *self };
        let mut out = Vec::new();
        for key in Self::MODIFIER_KEYS {
            if self.is_down(key) {
                shown.track(&KeyEvent { key, press, flags: ModifierFlags::empty() });
                out.push(emit(key, press, shown.flags()));
            }
        }
        out
    }

    const MODIFIER_KEYS: [Key; 8] = [
        Key::ControlLeft, Key::ControlRight, Key::MetaLeft, Key::MetaRight,
        Key::AltLeft, Key::AltRight, Key::ShiftLeft, Key::ShiftRight,
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
            _ => false,
        }
    }

    #[must_use]
    pub fn flags(&self) -> ModifierFlags {
        let mut f = ModifierFlags::empty();
        f.set(ModifierFlags::CONTROL, self.control.any_held());
        f.set(ModifierFlags::COMMAND, self.meta.any_held());
        f.set(ModifierFlags::ALT,     self.alt.any_held());
        f.set(ModifierFlags::SHIFT,   self.shift.any_held());
        f
    }
}

fn emit(key: Key, press: PressType, flags: ModifierFlags) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press, flags })
}
```

`sweep` needs `HeldModifiers: Copy` for the leaving snapshot; it is all-bool, so `Clone, Copy` on it and `LeftRightPair`.

`held` lives on the root, not on typing: the flush reads it as typing is entered and left, so it has to outlive the layer.

## `handle` is unchanged (`state.rs`)

`handle` stays a bare dispatch. The passthrough and the held tracking are bound handlers, not something `handle` does.

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

## Root handlers (`sources.rs`, `handlers/`)

The two key catch-alls split on whether the key is a modifier:

```rust
pub struct AnyModifierKey;
impl EventTrigger for AnyModifierKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool { ev.key.is_modifier() }
}

pub struct AnyNonModifierKey;
impl EventTrigger for AnyNonModifierKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool { !ev.key.is_modifier() }
}
```

No layer binds a modifier, so a modifier key falls to the root in every layer. `on_modifier` updates `held` there and passes the modifier through while a passthrough layer is active:

```rust
pub(crate) fn on_modifier(ev: &KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.held.track(ev);
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, root.held.flags())]
    } else {
        Vec::new()
    }
}
```

`maybe_passthru` passes a non-modifier through while a passthrough layer is active, and swallows it otherwise:

```rust
pub(crate) fn maybe_passthru(ev: &KeyEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    if root.layer().is_passthrough() {
        vec![emit(ev.key, ev.press, root.held.flags())]
    } else {
        Vec::new()
    }
}
```

A leafward binding still wins: a key the active layer binds runs the layer's command and never reaches these. They are the last resort, reached only when nothing leafward claimed the key.

## `set_layer` (`state.rs`)

Every transition replaces the active layer through `set_layer`, a method on `Mercury`, since the root owns both `layer` and `held`. It returns the flush for the transition handler to hand back. It checks the old and new layer independently: close if leaving a passthrough layer, open if entering one. A passthrough-to-passthrough change emits a close then an open, which nets to the held state.

```rust
impl Mercury {
    #[must_use = "the returned flush has to be emitted, or a held modifier is stranded down"]
    pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
        let mut fx = Vec::new();
        if self.layer.is_passthrough() {
            fx.extend(self.held.close());
        }
        self.layer = into.into();
        if self.layer.is_passthrough() {
            fx.extend(self.held.open());
        }
        fx
    }
}
```

`impl Into<Layer>` lets a caller pass the layer struct directly (`root.set_layer(HomeLayer {})`); a `derive_more::From` on `Layer` supplies the per-variant `From`.

`layer` is private and `set_layer` is its only mutator. Every handler is in another module (`handlers/*.rs`) and reaches the layer through `root.set_layer(...)` and the `layer()` getter, so no handler can write the field and skip the flush. The `#[must_use]` return catches a handler that drops the flush instead of returning it.

## `TypingLayer` (`state.rs`)

Typing binds only `escape`; everything else falls to the root. `escape` is a command (`cmd`-`escape` leaves to home), so binding it clobbers the root passthrough for escape, and a plain escape is re-emitted by hand.

```rust
#[bind(Key::Escape.down() => maybe_go_home)]
pub struct TypingLayer {}
```

`to_typing` enters through `set_layer`, returning its open:

```rust
pub(crate) fn to_typing<'a, P: Ascend<MercuryPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    node.parent.ascend_to::<MercuryPath>().set_layer(TypingLayer {})
}
```

`maybe_go_home` leaves through `set_layer` when `cmd` is held, returning its close, and otherwise passes the escape through:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.held.meta.any_held() {
        root.set_layer(HomeLayer {})
    } else {
        vec![emit(ev.key, ev.press, root.held.flags())]
    }
}
```

## Quit

`on_quit` returns `vec![MercuryEffect::Kill]` and no flush. Releasing the keyboard grab delivers the real key-ups to the app, so the held modifiers release themselves; a synthetic close would fire an up while the key is still physically down.

## Flags on emitted keys

A `CGEvent`'s modifier flags are baked in when it is created, from the event source's state, which lags a modifier posted microseconds earlier. So an emitted event states its own flags rather than trusting the source. The flags live on `KeyEvent`:

```rust
pub struct KeyEvent {
    pub key: Key,
    pub press: PressType,
    pub flags: ModifierFlags,
}
```

Whoever builds the event sets its flags:

- a passed-through key (`maybe_passthru`, `on_modifier`, `maybe_go_home`'s plain escape): `held.flags()`.
- a flush (`open`/`close`): the per-event `shown.flags()` the sweep stamps.
- a synthesized chord (`refresh`'s `cmd`-`r`): its own flags, independent of `held`.

The emitter applies the event's flags and nothing else:

```rust
fn emit(&self, ev: &KeyEvent) {
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(ev.flags));
}
```

`ModifierFlags` is a bitset in `freddie_keys` (`CONTROL`/`COMMAND`/`ALT`/`SHIFT`); `to_cg` maps it to `CGEventFlags`, built from the existing per-key `flag_of`.

## `fn`

`fn` is not a `Key` variant, so `held` cannot track it and a `fn`-modified passthrough is not caught up. Left to the Wispr fn-a follow-up.
