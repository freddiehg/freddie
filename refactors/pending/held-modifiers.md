# physical modifiers at the root, flushed at passthrough boundaries

The root owns a `Passthrough` struct: the physical modifier keys held, and a count of how many passthrough layers are active. `held` is updated on every key, in every state, unconditionally — it is just the truth about the keyboard, so no handler maintains it.

A passthrough layer (typing, paused) is one where held keys reach the app; a command layer swallows them. So the app's view of what is held should match physical reality while a passthrough layer is active and be empty otherwise, kept true by flushing at the boundary:

- Entering: emit a down for every held key (OPEN), so the app catches up.
- Leaving: emit an up for every held key (CLOSE), so the app forgets them (the command layer swallows their real ups; without this they stay stuck).

The flush is not written by hand at each transition, and it flushes only on the TRUE boundary. `Passthrough` hands out a guard, stored on the layer. Making a guard returns the opens — empty if a passthrough layer was already active (pausing over typing raises the count 1 to 2, opens nothing). Clearing a guard returns the closes — empty if another passthrough layer is still active. So the count, not each handler, decides whether a flush happens, and only 0↔1 flushes.

`MercuryEffect` is `#[must_use]`, and both guard operations return `Vec<MercuryEffect>`, so a handler cannot drop the flush; it has to return it, and it flows out.

Not in scope, separate follow-ups: keeping the ORIGINAL event so injected inline flags survive (the Wispr fn-a fix, the tap), and handling ALL keys at the root (`root-passthrough.md`).

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
    /// physical state, it does not change it. Only `track` (a real key event) changes what is held.
    #[must_use] fn open(&self)  -> Vec<MercuryEffect> { self.flush(PressType::Down) }
    #[must_use] fn close(&self) -> Vec<MercuryEffect> { self.flush(PressType::Up) }

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

    /// The modifier flags implied by what is held; the emitter stamps these. `ModifierFlags` is a
    /// portable bitset in `freddie_keys`, mapped to `CGEventFlags` at the macOS boundary.
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

`open`/`close` are private: only `Passthrough` calls them, on the boundary. `fn` is not a `Key` variant, so it is punted; `caps_lock` is a plain bool (no left/right).

## The `Passthrough` struct and its guard (`state.rs`)

```rust
#[derive(Debug, Default)]
pub struct Passthrough {
    pub held: HeldModifiers,
    active: u8, // how many passthrough layers are live; nested (paused over typing) is > 1
}

/// Proof that one passthrough layer is live, stored on `TypingLayer`/`Paused`. Made by `guard`,
/// consumed by `clear` — both return the flush. `#[must_use]`, so it is not dropped on the floor.
#[must_use]
pub struct PassthroughGuard;

impl Passthrough {
    /// Enter a passthrough layer. Opens the held keys iff this is the first (0 -> 1); empty if one
    /// was already active.
    pub fn guard(&mut self) -> (PassthroughGuard, Vec<MercuryEffect>) {
        self.active += 1;
        let opens = if self.active == 1 { self.held.open() } else { Vec::new() };
        (PassthroughGuard, opens)
    }

    /// Leave a passthrough layer, consuming its guard. Closes the held keys iff this was the last
    /// (1 -> 0); empty otherwise.
    #[must_use]
    pub fn clear(&mut self, _guard: PassthroughGuard) -> Vec<MercuryEffect> {
        self.active -= 1;
        if self.active == 0 { self.held.close() } else { Vec::new() }
    }

    /// The close a shutdown owes the app: close held keys iff we are in a passthrough layer.
    #[must_use]
    pub fn closing_flush(&self) -> Vec<MercuryEffect> {
        if self.active > 0 { self.held.close() } else { Vec::new() }
    }
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

After (`passthrough` owns held + the count):

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

## Track always (`state.rs`, `Mercury::handle`)

Before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

After:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    if let MercuryEvent::Key(ev) = event {
        self.passthrough.held.track(ev); // physical truth, whatever the layer
    }
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

## The layers hold a guard, lose their own held (`state.rs`)

`SetOfHeldKeys`, the old `HeldModifiers { cmd, alt }`, and every hand-written release go away; the catch-alls stay only to pass in-layer keys through. Before:

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

After (each holds a `PassthroughGuard`; `Paused::new` is gone — building it needs a guard, which only a handler with `&mut Passthrough` can make):

```rust
#[bind(AnyKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    guard: PassthroughGuard,
}

#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyKey => pass_through,
)]
pub struct TypingLayer {
    guard: PassthroughGuard,
}
```

Typing's catch-all is now a plain pass-through (`modify_held_and_pass_through` -> `pass_through`):

```rust
pub(crate) fn pass_through(ev: &KeyEvent, _node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}
```

## `Power` transitions make and clear the guard (`state.rs`)

`pause`/`unpause`/`toggle` take `&mut Passthrough`, move the layer as before, and make or clear the guard, returning its flush. Before:

```rust
pub(crate) const fn pause(&mut self) {
    *self = match std::mem::replace(self, Self::Paused(Paused::new(Layer::Home(HomeLayer {})))) {
        Self::Unpaused(u) => Self::Paused(Paused::new(u.layer)),
        already @ Self::Paused(_) => already,
    };
}
// unpause, toggle similar
```

After (placeholder is `Unpaused`, which needs no guard):

```rust
fn placeholder() -> Power { Power::Unpaused(Unpaused { layer: Layer::Home(HomeLayer {}) }) }

#[must_use]
pub fn pause(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::replace(self, Self::placeholder()) {
        Self::Unpaused(u) => {
            let (guard, opens) = passthrough.guard();
            *self = Self::Paused(Paused { layer: u.layer, guard });
            opens
        }
        already @ Self::Paused(_) => { *self = already; Vec::new() }
    }
}

#[must_use]
pub fn unpause(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::replace(self, Self::placeholder()) {
        Self::Paused(p) => {
            *self = Self::Unpaused(Unpaused { layer: p.layer });
            passthrough.clear(p.guard)
        }
        already @ Self::Unpaused(_) => { *self = already; Vec::new() }
    }
}

#[must_use]
pub fn toggle(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::replace(self, Self::placeholder()) {
        Self::Unpaused(u) => {
            let (guard, opens) = passthrough.guard();
            *self = Self::Paused(Paused { layer: u.layer, guard });
            opens
        }
        Self::Paused(p) => {
            *self = Self::Unpaused(Unpaused { layer: p.layer });
            passthrough.clear(p.guard)
        }
    }
}
```

## Handlers thread the flush out (`home.rs`, `typing.rs`, `toggle.rs`)

`&root.power` and `&mut root.passthrough` are disjoint fields, so both borrows are fine. `on_toggle`:

```rust
pub(crate) fn on_toggle(_ev: &ToggleEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.power.toggle(&mut root.passthrough)
}
```

`pause` (`home.rs`):

```rust
pub(crate) fn pause(_ev: &KeyEvent, node: Node<HomeLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend().ascend_to::<MercuryPath>();
    root.power.pause(&mut root.passthrough)
}
```

`to_typing` (`home.rs`) makes typing's guard directly (it is a layer switch, not a pause):

```rust
pub(crate) fn to_typing<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend().ascend_to::<MercuryPath>();
    let (guard, opens) = root.passthrough.guard();
    *root.power.layer_mut() = Layer::Typing(TypingLayer { guard });
    opens
}
```

`maybe_go_home` (`typing.rs`) takes typing's guard back out on the way to home:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.passthrough.held.meta.any_held() {
        let Layer::Typing(TypingLayer { guard }) =
            std::mem::replace(root.power.layer_mut(), Layer::Home(HomeLayer {}))
        else { unreachable!("maybe_go_home only runs in typing") };
        root.passthrough.clear(guard)
    } else {
        vec![MercuryEffect::Emit(ev.clone())] // plain escape passes through
    }
}
```

Paused's catch-all (`toggle.rs`) reads `root.passthrough.held` for the chord and unpauses through `Power::unpause`:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if ev.key == Key::KeyP
        && ev.press == PressType::Down
        && root.passthrough.held.meta.any_held()
        && root.passthrough.held.alt.any_held()
    {
        return root.power.unpause(&mut root.passthrough); // closes on the boundary; p swallowed
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
```

## Quit and drop

The flush leaves the model because a fn with `&mut Path` returns `Vec<MercuryEffect>` — the handler signature `fn(&Event, Node<P, ()>) -> Vec<MercuryEffect>` guarantees it — and the guard operations are `#[must_use]` on top. Two places have `&mut` access without that return, and both must close held keys or leave modifiers stuck downstream after mercury is gone:

- Quit. `on_quit` has `&mut Mercury` and returns effects, so it closes first:

```rust
pub(crate) fn on_quit(_ev: &QuitEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    let mut effects = root.passthrough.closing_flush();
    effects.push(MercuryEffect::Kill);
    effects
}
```

- Drop. `Drop::drop(&mut self)` returns nothing and `Mercury` does not hold the emitter, so it cannot emit. The shutdown sequence that owns the emitter (`main.rs`) does it: call `passthrough.closing_flush()` and emit its ups through the `Emitter` before the tree is dropped. "If possible" is exactly this — at the orchestration level, not inside `Drop`. A `SIGKILL` gets neither.

## Flags are `held.flags()`

The emitter rebuilds its own modifier picture per emitted key (`self.flags` / `next_flags` in `macos.rs`). Delete that and stamp `passthrough.held.flags()`, so `held` is the one owner. `held` is physical, so its flags are always the true modifier state; `to_cg` (from the existing per-key `flag_of`) maps to `CGEventFlags` at the boundary.
