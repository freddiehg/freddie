# physical modifiers at the root, flushed at passthrough boundaries

`held` models the physical modifier keys currently down. It is updated on every key, in every state, unconditionally: it is just the truth about the keyboard, so no handler maintains it.

A passthrough layer (typing, paused) is one where held keys reach the app; a command layer swallows them. So the app's view of what is held should match physical reality while in a passthrough layer and be empty otherwise, kept true by a flush at the boundary:

- Entering a passthrough layer: OPEN, emit a down for every held key, so the app catches up (in the command layer we came from these were swallowed).
- Leaving a passthrough layer: CLOSE, emit an up for every held key, so the app forgets them (the command layer we enter swallows their real ups, so without this they stay stuck).

The flush is not written by hand at each transition. Every change to `power` (a layer switch, a pause, an unpause) goes through one primitive, `change_power`, which flushes iff the change crossed the passthrough boundary and RETURNS those effects. `MercuryEffect` is `#[must_use]`, so a handler cannot drop the flush; it has to return it, and it flows out.

Not in scope, separate follow-ups: keeping the ORIGINAL event so injected inline flags survive (the Wispr fn-a fix, the tap), and handling ALL keys at the root (`root-passthrough.md`).

## Effects are `#[must_use]` (`lib.rs`)

Before:

```rust
pub enum MercuryEffect {
    Emit(KeyEvent),
    Foreground(App),
    // ...
}
```

After:

```rust
#[must_use]
pub enum MercuryEffect {
    Emit(KeyEvent),
    Foreground(App),
    // ...
}
```

## The struct (`state.rs`)

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
    /// Record a modifier key event. Non-modifiers are ignored. Called on every key, always.
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

    /// Emit a DOWN for every held key. Called on ENTERING a passthrough layer. Reads only.
    #[must_use]
    pub fn open(&self) -> Vec<MercuryEffect> { self.flush(PressType::Down) }

    /// Emit an UP for every held key. Called on LEAVING a passthrough layer. Reads only.
    #[must_use]
    pub fn close(&self) -> Vec<MercuryEffect> { self.flush(PressType::Up) }

    // `&self`: open/close read the physical state and emit; they do NOT change what is held. Only
    // `track` (a real key event) does.
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

`fn` is not a `Key` variant, so it is punted; `caps_lock` is a plain bool (no left/right).

## The root gains `held` (`state.rs`)

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
    pub held: HeldModifiers,
    #[resolve_into]
    pub power: Power,
}
```

`Default` for `Mercury` adds `held: HeldModifiers::default(),` in the same position.

## Track always; one primitive for every power change (`state.rs`, `Mercury`)

`handle` tracks on every key, unconditionally. `change_power` is the single door for changing `power`; it flushes iff the change crossed the passthrough boundary, and is `#[must_use]` so the flush cannot be dropped. `handle`, before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

After (plus the two new methods):

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    if let MercuryEvent::Key(ev) = event {
        self.held.track(ev); // physical truth, whatever the layer
    }
    bind::dispatch::<MercuryStruct, Self>(self, event)
}

/// Change `power`, then flush held keys iff that crossed the passthrough boundary. The ONLY way
/// a handler should mutate `power`. `#[must_use]`: the returned flush has to be returned onward.
#[must_use]
pub fn change_power(&mut self, f: impl FnOnce(&mut Power)) -> Vec<MercuryEffect> {
    let was = self.layer_is_passthrough();
    f(&mut self.power);
    match (was, self.layer_is_passthrough()) {
        (false, true) => self.held.open(),  // entered a passthrough layer
        (true, false) => self.held.close(), // left one
        _ => Vec::new(),                     // no boundary crossed (command->command, etc.)
    }
}

/// A passthrough layer is one whose keys reach the app: typing, or any paused state.
const fn layer_is_passthrough(&self) -> bool {
    matches!(self.power, Power::Paused(_)) || matches!(self.power.layer(), Layer::Typing(_))
}
```

## The layers keep their catch-alls but stop tracking and releasing (`state.rs`, `typing.rs`, `toggle.rs`)

`SetOfHeldKeys`, the old `HeldModifiers { cmd, alt }`, and every hand-written release go away; the catch-alls stay only to pass in-layer keys through. Before (`state.rs`):

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

After:

```rust
#[bind(AnyKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
}

impl Paused {
    const fn new(layer: Layer) -> Self { Self { layer } }
}

#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyKey => pass_through,
)]
pub struct TypingLayer {}
```

Typing's catch-all is now a plain pass-through. Before (`modify_held_and_pass_through`, `typing.rs`):

```rust
pub(crate) fn modify_held_and_pass_through(ev: &KeyEvent, mut node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    if matches!(ev.key, Key::MetaLeft | Key::MetaRight) {
        node.parent.get_mut().held.cmd = (ev.press == PressType::Down).then_some(ev.key);
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
```

After (renamed `pass_through`):

```rust
pub(crate) fn pass_through(ev: &KeyEvent, _node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Emit(ev.clone())]
}
```

## Transitions go through `change_power` (`home.rs`, `typing.rs`, `toggle.rs`)

Every enter/leave routes through `change_power`, so the flush happens automatically and its `#[must_use]` result is what the handler returns.

`to_typing` (`home.rs`), before:

```rust
*node.parent.ascend().get_mut() = Layer::Typing(TypingLayer::default());
Vec::new()
```

After:

```rust
node.parent
    .ascend()
    .ascend_to::<MercuryPath>()
    .change_power(|p| *p.layer_mut() = Layer::Typing(TypingLayer {}))
```

`pause` (`home.rs`), before:

```rust
node.parent.ascend()... .pause();
Vec::new()
```

After:

```rust
node.parent.ascend().ascend_to::<MercuryPath>().change_power(Power::pause)
```

`on_toggle` (`toggle.rs`), before:

```rust
let root = node.parent;
root.power.toggle();
Vec::new()
```

After:

```rust
node.parent.change_power(Power::toggle)
```

`maybe_go_home` (`typing.rs`), before:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, mut node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let cmd = node.parent.get_mut().held.cmd;
    if let Some(cmd) = cmd {
        let mut layer = node.parent.into_parent();
        go_home(&mut layer);
        vec![MercuryEffect::Emit(KeyEvent { key: cmd, press: PressType::Up })]
    } else {
        vec![MercuryEffect::Emit(ev.clone())]
    }
}
```

After:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.held.meta.any_held() {
        // cmd-escape: leave to home; change_power emits the close for the held keys.
        root.change_power(|p| *p.layer_mut() = Layer::Home(HomeLayer {}))
    } else {
        vec![MercuryEffect::Emit(ev.clone())] // plain escape passes through
    }
}
```

Paused's catch-all (`toggle.rs`), before:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, mut node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let down = ev.press == PressType::Down;
    let paused = node.parent.get_mut();
    match ev.key {
        Key::MetaLeft | Key::MetaRight => paused.held.cmd = down.then_some(ev.key),
        Key::AltLeft | Key::AltRight => paused.held.alt = down.then_some(ev.key),
        _ => {}
    }
    if ev.key == Key::KeyP
        && down
        && let (Some(cmd), Some(alt)) = (paused.held.cmd, paused.held.alt)
    {
        node.parent.ascend_to::<PowerPath>().get_mut().unpause();
        return vec![
            MercuryEffect::Emit(KeyEvent { key: cmd, press: PressType::Up }),
            MercuryEffect::Emit(KeyEvent { key: alt, press: PressType::Up }),
        ];
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
```

After:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if ev.key == Key::KeyP
        && ev.press == PressType::Down
        && root.held.meta.any_held()
        && root.held.alt.any_held()
    {
        // cmd-alt-p: unpause; change_power emits the close; the p is swallowed.
        return root.change_power(Power::unpause);
    }
    vec![MercuryEffect::Emit(ev.clone())]
}
```

`Power::pause`/`unpause`/`toggle` keep their signatures (`&mut self`), so they pass straight to `change_power`; only `Paused::new` changes, now taking just the layer.

## The invariant, and the two exceptions (quit, drop)

The flush can always leave the model because of a property mercury already has: a fn with `&mut Path` returns `Vec<MercuryEffect>`. Every handler is `fn(&Event, Node<P, ()>) -> Vec<MercuryEffect>` — the `Node` carries the mutable path, the return carries the effects — so anything that mutates the tree has a channel for what that mutation must emit. `change_power` is `#[must_use]` on top of that, so the flush is not just returnable but un-droppable.

Two places have `&mut` access WITHOUT that return, and both must close the held keys or leave modifiers stuck downstream after mercury is gone:

- Quit. `on_quit` has `&mut Mercury` and returns effects, so it just closes first. A shared helper does the "close iff we are currently passthrough" (the same boundary test), before `Kill`:

```rust
// Mercury
#[must_use]
pub(crate) fn closing_flush(&self) -> Vec<MercuryEffect> {
    if self.layer_is_passthrough() { self.held.close() } else { Vec::new() }
}

// on_quit (quit.rs), after
pub(crate) fn on_quit(_ev: &QuitEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    let mut effects = root.closing_flush();
    effects.push(MercuryEffect::Kill);
    effects
}
```

- Drop. `Drop::drop(&mut self)` returns nothing and `Mercury` does not hold the emitter, so it cannot emit. The flush has to be done by the shutdown sequence that owns the emitter (`main.rs`): call `closing_flush` and emit its ups through the `Emitter` before the tree is dropped. "If possible" is exactly this — possible at the orchestration level, not inside `Drop`. A `SIGKILL` gets neither; nothing can.

## Flags are `held.flags()`

The emitter rebuilds its own modifier picture per emitted key (`self.flags` / `next_flags` in `macos.rs`). Delete that and stamp `held.flags()`, so `held` is the one owner. `held` is physical, so `held.flags()` is always the true modifier state; `to_cg` (built from the existing per-key `flag_of`) maps it to `CGEventFlags` at the boundary.
