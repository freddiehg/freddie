# physical modifiers at the root

The root owns a `Passthrough` struct: the physical modifier keys held, and a count of how many passthrough layers are active. `held` is updated on every key, in every state, unconditionally — it is just the truth about the keyboard, so no handler maintains it.

Modifier keys do not go through any layer's catch-all (it is `AnyNonModifierKey` now). The root owns them: it passes a modifier through while a passthrough layer is active (count > 0) and swallows it otherwise, and it keeps the app's modifier state in sync across a layer change by flushing at the boundary:

- Entering a passthrough layer (count 0 -> 1): OPEN, emit a down for every held key, so the app catches up to physical reality.
- Leaving the last one (count 1 -> 0): CLOSE, emit an up for every held key, so the app forgets them (the command layer swallows their real ups; without this they stay stuck).

Both flushes live in one place, `Mercury::handle`, which diffs the count around each dispatch. Nobody calls a close; the guard's `Drop` lowers the count when a layer is dropped, and `handle` sees the count hit 0 and emits. Open and close are symmetric — same place, same trigger, opposite direction. On quit or drop the guards drop OUTSIDE any dispatch, so `handle` never flushes there, which is correct: releasing the keyboard grab lets the real key-ups through.

Not in scope, separate follow-ups: keeping the ORIGINAL event so injected inline flags survive (the Wispr fn-a fix, the tap), and moving the non-modifier passthrough to the root too (`root-passthrough.md`).

## Held modifiers (`state.rs`)

```rust
#[derive(Debug, Default)]
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

#[derive(Debug, Default)]
pub struct HeldModifiers {
    pub control: LeftRightPair,
    pub meta: LeftRightPair,
    pub alt: LeftRightPair,
    pub shift: LeftRightPair,
    pub caps_lock: bool,
}

impl HeldModifiers {
    /// Record a modifier key event. Non-modifiers are ignored.
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

    /// A DOWN for every held key, and an UP for every held key. Read-only: a flush emits the
    /// physical state, it does not change it. Only `track` (a real key event) does.
    fn open(&self)  -> Vec<MercuryEffect> { self.flush(PressType::Down) }
    fn close(&self) -> Vec<MercuryEffect> { self.flush(PressType::Up) }

    fn flush(&self, press: PressType) -> Vec<MercuryEffect> {
        let mut out = Vec::new();
        for (pair, left, right) in [
            (&self.control, Key::ControlLeft, Key::ControlRight),
            (&self.meta,    Key::MetaLeft,    Key::MetaRight),
            (&self.alt,     Key::AltLeft,     Key::AltRight),
            (&self.shift,   Key::ShiftLeft,   Key::ShiftRight),
        ] {
            if pair.left  { out.push(emit(left, press)); }
            if pair.right { out.push(emit(right, press)); }
        }
        if self.caps_lock { out.push(emit(Key::CapsLock, press)); }
        out
    }

    /// The modifier flags implied by what is held; the emitter stamps these on what it emits.
    /// `ModifierFlags` is a portable bitset in `freddie_keys`, mapped to `CGEventFlags` at the
    /// macOS boundary.
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

fn emit(key: Key, press: PressType) -> MercuryEffect {
    MercuryEffect::Emit(KeyEvent { key, press })
}
```

`open`/`close` are private: only `handle` calls them, on the boundary. `fn` is not a `Key` variant, so it is punted; `caps_lock` is a plain bool.

## The `Passthrough` struct and its guard (`state.rs`)

```rust
use std::cell::Cell;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct Passthrough {
    pub held: HeldModifiers,
    active: Rc<Cell<u8>>, // how many passthrough layers are live; shared with the guards
}

/// Proof that one passthrough layer is live, stored on `TypingLayer`/`Paused`. Its `Drop` is the
/// ONLY place the count goes down, so reassigning a layer or tearing down the tree keeps the count
/// correct with nothing to remember. `Rc<Cell>` because the guard outlives the borrow of
/// `Passthrough` (it lives on the layer); single-threaded dispatch, so `Rc`/`Cell`, not `Arc`.
pub struct PassthroughGuard(Rc<Cell<u8>>);

impl Drop for PassthroughGuard {
    fn drop(&mut self) { self.0.set(self.0.get() - 1); }
}

impl Passthrough {
    /// Enter a passthrough layer: raise the count, hand back a guard to store on the layer. No
    /// effects — `handle` sees the count cross 0 -> 1 and emits the open. The guard MUST be stored,
    /// or its `Drop` lowers the count right back.
    pub fn guard(&mut self) -> PassthroughGuard {
        self.active.set(self.active.get() + 1);
        PassthroughGuard(Rc::clone(&self.active))
    }

    fn count(&self) -> u8 { self.active.get() }
}
```

## The root gains `passthrough` (`state.rs`)

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
    pub passthrough: Passthrough,
    #[resolve_into]
    pub power: Power,
}
```

`Default` adds `passthrough: Passthrough::default(),`.

## `handle` owns modifiers and the boundary flush (`state.rs`)

Track every key. Pass a modifier through iff a passthrough layer is active. Then diff the count around dispatch and emit the open or close. Before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

After:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let mut out = Vec::new();

    if let MercuryEvent::Key(ev) = event {
        self.passthrough.held.track(ev); // physical truth, whatever the layer
        // No layer binds a modifier (AnyNonModifierKey), so the root passes it through while a
        // passthrough layer is active and swallows it otherwise.
        if ev.key.is_modifier() && self.passthrough.count() > 0 {
            out.push(MercuryEffect::Emit(ev.clone()));
        }
    }

    let before = self.passthrough.count();
    let effects = bind::dispatch::<MercuryStruct, Self>(self, event).unwrap_or_default();
    match (before, self.passthrough.count()) {
        (0, n) if n > 0 => out.extend(self.passthrough.held.open()),  // entered a passthrough layer
        (n, 0) if n > 0 => out.extend(self.passthrough.held.close()), // left the last one
        _ => {}
    }

    out.extend(effects);
    Some(out)
}
```

(`Key::is_modifier` is a small helper in `freddie_keys`. A modifier event never triggers a layer change, so the modifier-passthrough and the boundary flush never both fire for one event.)

## The layers hold a guard, lose their own held (`state.rs`)

The catch-all trigger is renamed `AnyKey` -> `AnyNonModifierKey` (matches every key EXCEPT a modifier), and each layer holds a `PassthroughGuard`. `SetOfHeldKeys` and the old `HeldModifiers { cmd, alt }` are deleted. Before:

```rust
#[derive(Debug, Default)]
pub struct HeldModifiers {           // old, on Paused
    pub cmd: Option<Key>,
    pub alt: Option<Key>,
}

#[bind(AnyKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    pub held: HeldModifiers,
}

impl Paused {
    const fn new(layer: Layer) -> Self {
        Self { layer, held: HeldModifiers { cmd: None, alt: None } }
    }
}

pub struct SetOfHeldKeys { pub cmd: Option<Key> }

#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyKey => modify_held_and_pass_through,
)]
pub struct TypingLayer {
    pub held: SetOfHeldKeys,
}
```

After (constructors kept, now taking the guard, so construction works across modules despite the private field):

```rust
#[bind(AnyNonModifierKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    guard: PassthroughGuard,
}

impl Paused {
    pub(crate) fn new(layer: Layer, guard: PassthroughGuard) -> Self {
        Self { layer, guard }
    }
}

#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyNonModifierKey => pass_through,
)]
pub struct TypingLayer {
    guard: PassthroughGuard,
}

impl TypingLayer {
    pub(crate) fn new(guard: PassthroughGuard) -> Self {
        Self { guard }
    }
}
```

The catch-all is now a plain pass-through (`modify_held_and_pass_through` -> `pass_through`), and it only ever sees non-modifier keys:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, _node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}
```

## `Power` transitions: pull the layer, rebuild (`state.rs`)

No `mem::replace` dance. `Layer` gets a `#[default]` of `Home`, so the layer can be moved out with `mem::take`; the arm is then rebuilt around it. Entering makes a guard; leaving drops the old arm, whose guard `Drop` lowers the count. Rebuilding an arm you were already in just drops the old guard and makes a new one — net zero on the count, no flush. Before:

```rust
pub(crate) const fn pause(&mut self) {
    *self = match std::mem::replace(self, Self::Paused(Paused::new(Layer::Home(HomeLayer {})))) {
        Self::Unpaused(u) => Self::Paused(Paused::new(u.layer)),
        already @ Self::Paused(_) => already,
    };
}
// unpause, toggle similar
```

After (`Layer` derives `Default` with `#[default] Home(HomeLayer)`):

```rust
pub fn pause(&mut self, passthrough: &mut Passthrough) {
    let layer = std::mem::take(self.layer_mut());
    *self = Self::Paused(Paused::new(layer, passthrough.guard()));
}

pub fn unpause(&mut self) {
    let layer = std::mem::take(self.layer_mut());
    *self = Self::Unpaused(Unpaused { layer }); // old arm dropped -> its guard (if Paused) Drops -> count--
}

pub fn toggle(&mut self, passthrough: &mut Passthrough) {
    let was_paused = matches!(self, Self::Paused(_));
    let layer = std::mem::take(self.layer_mut());
    *self = if was_paused {
        Self::Unpaused(Unpaused { layer })
    } else {
        Self::Paused(Paused::new(layer, passthrough.guard()))
    };
}
```

## Handlers (`home.rs`, `typing.rs`, `toggle.rs`)

They no longer thread any flush — `handle` does it. They return their own effects (usually nothing). `on_toggle`:

```rust
pub(crate) fn on_toggle(_ev: &ToggleEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.power.toggle(&mut root.passthrough);
    Vec::new()
}
```

`pause` (`home.rs`):

```rust
pub(crate) fn pause(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend().ascend_to::<MercuryPath>();
    root.power.pause(&mut root.passthrough);
    Vec::new()
}
```

`to_typing` (`home.rs`) makes typing's guard directly (a layer switch, not a pause):

```rust
pub(crate) fn to_typing<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend().ascend_to::<MercuryPath>();
    let guard = root.passthrough.guard();
    *root.power.layer_mut() = Layer::Typing(TypingLayer::new(guard));
    Vec::new()
}
```

`maybe_go_home` (`typing.rs`) just reassigns; the old `TypingLayer`'s guard `Drop` lowers the count and `handle` closes:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.passthrough.held.meta.any_held() {
        *root.power.layer_mut() = Layer::Home(HomeLayer {}); // old TypingLayer's guard drops -> count--
        Vec::new()
    } else {
        vec![MercuryEffect::Emit(ev.clone())] // plain escape passes through
    }
}
```

Paused's catch-all (`toggle.rs`) reads `root.passthrough.held` for the chord and unpauses:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if ev.key == Key::KeyP
        && ev.press == PressType::Down
        && root.passthrough.held.meta.any_held()
        && root.passthrough.held.alt.any_held()
    {
        root.power.unpause(); // guard drops -> count-- -> handle closes; the p is swallowed
        return Vec::new();
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
```

## Quit and drop do nothing

No flush on the way out. A close is only needed because a LIVE mercury swallows the real key-up in a command layer; on quit or drop the `Interceptor` releases the keyboard grab, so the keys still physically held deliver their real ups directly to the OS and app. `on_quit` returns `vec![MercuryEffect::Kill]` unchanged. A synthetic close here would be worse than nothing — it would fire an up while the key is still physically down, ahead of the real one. (Consistent with the model: the guards drop as the tree is torn down, outside any dispatch, so `handle` never flushes there.)

## `AnyNonModifierKey` (`bind` / mercury)

`AnyKey` was a unit trigger matching everything. `AnyNonModifierKey` matches every key except a modifier, so a modifier never reaches a layer's catch-all and always falls to the root:

```rust
pub struct AnyNonModifierKey;

impl EventTrigger for AnyNonModifierKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        !ev.key.is_modifier()
    }
}
```

## Flags are `held.flags()`

The emitter rebuilds its own modifier picture per emitted key (`self.flags` / `next_flags` in `macos.rs`). Delete that and stamp `passthrough.held.flags()`, so `held` is the one owner. `held` is physical, so its flags are always the true modifier state; `to_cg` (from the existing per-key `flag_of`) maps to `CGEventFlags` at the boundary.
