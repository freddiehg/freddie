# physical modifiers at the root

The root owns a `Passthrough` struct: the physical modifier keys held, and a count of how many passthrough layers are active. `held` is updated on every key, in every state, unconditionally — it is just the truth about the keyboard, so no handler maintains it.

Modifier keys do not go through any layer's catch-all (it is `AnyNonModifierKey` now). The root owns them: it passes a modifier through while a passthrough layer is active (count > 0) and swallows it otherwise.

Entering or leaving a passthrough layer syncs the app's modifier view, and the sync is a RETURNED value, never an observed side effect:

- Entering (count 0 -> 1): constructing a `PassthroughGuard` returns an OPEN — a down for every held key — so the app catches up.
- Leaving the last one (count 1 -> 0): the guard's `close` returns a CLOSE — an up for every held key — so the app forgets them (the command layer swallows their real ups; without this they stay stuck).

The transition handler returns that flush, and it flows out through `handle`'s return like any other effect (`dispatch_event` sends whatever `handle` returns). The guard's `Drop` lowers the count so a torn-down or reassigned layer stays correct, but the FLUSH is a return value from `enter`/`leave`, not something `handle` infers from the count.

On quit or drop the guards drop outside any transition, so no flush is produced — correct, because releasing the keyboard grab lets the real key-ups through.

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

    /// The modifier flags implied by what is held. See "Setting the flags" below.
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
    MercuryEffect::Emit(KeyEvent { key, press }, ModifierFlags::empty()) // flags stamped in handle
}
```

`open`/`close` are private: only `enter`/`leave` call them. `fn` is not a `Key` variant, so it is punted; `caps_lock` is a plain bool.

## The `Passthrough` struct: enter and leave return the flush (`state.rs`)

```rust
use std::cell::Cell;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct Passthrough {
    pub held: HeldModifiers,
    active: Rc<Cell<u8>>, // how many passthrough layers are live; shared with the guards
}

/// Proof that one passthrough layer is live, stored on `TypingLayer`/`Paused`. Constructing one
/// enters a passthrough layer; consuming it (`close`) leaves. `Rc<Cell>` because the guard outlives
/// the borrow of `Passthrough` (it lives on the layer), so it owns a handle to the count;
/// single-threaded dispatch, so `Rc`/`Cell`, not `Arc`.
pub struct PassthroughGuard(Rc<Cell<u8>>);

impl PassthroughGuard {
    /// Enter a passthrough layer. The constructor IS the entry: it raises the count and RETURNS
    /// the open (empty unless this is the first, 0 -> 1). Store the guard on the layer.
    pub(crate) fn new(passthrough: &mut Passthrough) -> (Self, Vec<MercuryEffect>) {
        passthrough.active.set(passthrough.active.get() + 1);
        let opens = if passthrough.active.get() == 1 { passthrough.held.open() } else { Vec::new() };
        (Self(Rc::clone(&passthrough.active)), opens)
    }

    /// Leave a passthrough layer: RETURN the close (empty unless this is the last, 1 -> 0), then
    /// drop `self`, lowering the count. The less-clean half of the pair — the caller had to pull
    /// the guard out of the layer to consume it here.
    #[must_use]
    pub(crate) fn close(self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
        if passthrough.active.get() == 1 { passthrough.held.close() } else { Vec::new() }
        // `self` drops at the end of scope -> count--
    }
}

impl Drop for PassthroughGuard {
    /// The count's safety net: reassigning a layer or tearing down the tree drops the guard and
    /// lowers the count with nothing to remember. Cannot emit the close (no return), which is why
    /// `close` exists and is called on the leave path. Ordinary `Drop`, so the tree stays droppable.
    fn drop(&mut self) { self.0.set(self.0.get() - 1); }
}

impl Passthrough {
    fn count(&self) -> u8 { self.active.get() }
}
```

Construction hands its caller a hot potato. `PassthroughGuard::new` returns the opens as a `#[must_use]` `Vec<MercuryEffect>` — a linear obligation the caller cannot drop and must thread out to `handle` and on to the emitter. That side is clean, and the compiler helps.

Doing the same on the way out is the open problem. `Drop` has no return, so dropping the guard cannot hand anyone the close effects — there is no potato to pass. A scoped-closure (CPS) pattern, which is how `thread::scope` gets compile-time linearity, cannot rescue it: that works only when the guarded region is fully bracketed by one call so the linear value never escapes, and a passthrough layer is a state-machine mode entered on one event and left on a later one, living in the state TREE across many event-loop iterations. So the guard MUST escape into the tree, exactly what the scoped closure forbids. So we accept the asymmetry: `close(self)` returns the close hot-potato and the leave path calls it by discipline (`#[must_use]`, so its result cannot be dropped, but nothing forces the call), while `Drop` only keeps the count correct. The compiler enforces the opens on construction; the closes on the way out are convention.

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

## `handle` tracks, and passes modifiers through (`state.rs`)

`handle` tracks every key and, because no layer binds a modifier, passes a modifier through while a passthrough layer is active. It does NOT compute any flush — the flush is returned by the transition handlers below and rides out in `dispatch`'s effects. Before:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    bind::dispatch::<MercuryStruct, Self>(self, event)
}
```

After:

```rust
pub fn handle(&mut self, event: &MercuryEvent) -> Option<Vec<MercuryEffect>> {
    let mut effects = Vec::new();

    if let MercuryEvent::Key(ev) = event {
        self.passthrough.held.track(ev); // physical truth, whatever the layer
        if ev.key.is_modifier() && self.passthrough.count() > 0 {
            effects.push(emit(ev.key, ev.press)); // pass the modifier through
        }
    }

    effects.extend(bind::dispatch::<MercuryStruct, Self>(self, event).unwrap_or_default());
    stamp_flags(&mut effects, self.passthrough.held.flags()); // see "Setting the flags"
    Some(effects)
}
```

(`Key::is_modifier` is a small helper in `freddie_keys`. A modifier event never triggers a layer change, so passing it through never coincides with a flush.)

## The layers hold a guard, lose their own held (`state.rs`)

The catch-all trigger is renamed `AnyKey` -> `AnyNonModifierKey`, and each layer holds a `PassthroughGuard` (field `pub(crate)` so a handler in another module can build the layer). `SetOfHeldKeys` and the old `HeldModifiers { cmd, alt }` are deleted. Before:

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
#[bind(AnyNonModifierKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    pub(crate) guard: PassthroughGuard,
}

#[bind(
    Key::Escape.down() => maybe_go_home,
    AnyNonModifierKey => pass_through,
)]
pub struct TypingLayer {
    pub(crate) guard: PassthroughGuard,
}
```

The catch-all is now a plain pass-through (`modify_held_and_pass_through` -> `pass_through`), and only ever sees non-modifier keys:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, _node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    vec![emit(ev.key, ev.press)]
}
```

## `Power` transitions call enter/leave and return the flush (`state.rs`)

`Power` derives `Default` (`#[default] Unpaused` with a `Home` layer) so `mem::take` can move it; the arm is rebuilt around the taken value. Entering calls `enter` (opens); leaving calls `leave` with the old arm's guard (closes). Both RETURN the flush. Before:

```rust
pub(crate) const fn pause(&mut self) {
    *self = match std::mem::replace(self, Self::Paused(Paused::new(Layer::Home(HomeLayer {})))) {
        Self::Unpaused(u) => Self::Paused(Paused::new(u.layer)),
        already @ Self::Paused(_) => already,
    };
}
// unpause, toggle similar
```

After:

```rust
#[must_use]
pub fn pause(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::take(self) {
        Self::Unpaused(u) => {
            let (guard, opens) = PassthroughGuard::new(passthrough);
            *self = Self::Paused(Paused { layer: u.layer, guard });
            opens
        }
        already @ Self::Paused(_) => { *self = already; Vec::new() }
    }
}

#[must_use]
pub fn unpause(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::take(self) {
        Self::Paused(p) => {
            *self = Self::Unpaused(Unpaused { layer: p.layer });
            p.guard.close(passthrough)
        }
        already @ Self::Unpaused(_) => { *self = already; Vec::new() }
    }
}

#[must_use]
pub fn toggle(&mut self, passthrough: &mut Passthrough) -> Vec<MercuryEffect> {
    match std::mem::take(self) {
        Self::Unpaused(u) => {
            let (guard, opens) = PassthroughGuard::new(passthrough);
            *self = Self::Paused(Paused { layer: u.layer, guard });
            opens
        }
        Self::Paused(p) => {
            *self = Self::Unpaused(Unpaused { layer: p.layer });
            p.guard.close(passthrough)
        }
    }
}
```

## Handlers return the flush (`home.rs`, `typing.rs`, `toggle.rs`)

`on_toggle`:

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

`to_typing` (`home.rs`) — a layer switch, so it enters directly and returns the opens:

```rust
pub(crate) fn to_typing<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend().ascend_to::<MercuryPath>();
    let (guard, opens) = PassthroughGuard::new(&mut root.passthrough);
    *root.power.layer_mut() = Layer::Typing(TypingLayer { guard });
    opens
}
```

`maybe_go_home` (`typing.rs`) takes typing's guard out and returns the closes:

```rust
pub(crate) fn maybe_go_home(ev: &KeyEvent, node: Node<TypingLayerPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if root.passthrough.held.meta.any_held() {
        let Layer::Typing(typing) =
            std::mem::replace(root.power.layer_mut(), Layer::Home(HomeLayer {}))
        else { unreachable!("maybe_go_home only runs in typing") };
        typing.guard.close(&mut root.passthrough)
    } else {
        vec![emit(ev.key, ev.press)] // plain escape passes through
    }
}
```

Paused's catch-all (`toggle.rs`) reads `root.passthrough.held` for the chord and returns `unpause`'s closes:

```rust
pub(crate) fn pass_through(ev: &KeyEvent, node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    if ev.key == Key::KeyP
        && ev.press == PressType::Down
        && root.passthrough.held.meta.any_held()
        && root.passthrough.held.alt.any_held()
    {
        return root.power.unpause(&mut root.passthrough); // the p is swallowed; closes returned
    }
    vec![emit(ev.key, ev.press)]
}
```

## Quit and drop do nothing

A close is only needed because a LIVE mercury swallows the real key-up in a command layer; on quit or drop the `Interceptor` releases the grab, so the keys still physically held deliver their real ups directly to the OS and app. `on_quit` returns `vec![MercuryEffect::Kill]` unchanged. A synthetic close here would fire an up while the key is still physically down, ahead of the real one. (The guards drop as the tree is torn down, outside any transition, so no flush is produced — consistent.)

## `AnyNonModifierKey`

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

## Setting the flags on emitted keys, from `held`

Why stamp flags at all: a `CGEvent`'s modifier flags are baked in when it is created, from the event source's state, and that state only catches up once a posted modifier has been PROCESSED. So a chord posted back to back — `cmd` down then `r` — creates the `r` microseconds later carrying no `cmd`. The emitter today works around this by tracking the flags itself (`self.flags` / `next_flags` in `macos.rs`) and setting them on every posted event. That is a second modifier tracker, and it is the one to delete: `held` is the authoritative physical state, so the flags come from `held.flags()`.

Every emitted key carries the flags. `MercuryEffect::Emit` gains a `ModifierFlags`:

```rust
// before
Emit(KeyEvent),
// after
Emit(KeyEvent, ModifierFlags),
```

`handle` stamps `held.flags()` (physical, sampled once after `track`) onto every `Emit` it returns, so no handler thinks about flags:

```rust
fn stamp_flags(effects: &mut [MercuryEffect], flags: ModifierFlags) {
    for e in effects {
        if let MercuryEffect::Emit(_, f) = e {
            *f = flags;
        }
    }
}
```

The emitter maps `ModifierFlags` to `CGEventFlags` and stamps it, forcing the emitted key's OWN modifier bit to match its press. The own-bit override is what makes a CLOSE correct: `close` emits `cmd`-up while `cmd` is still physically held, so `held.flags()` has `cmd` set, but the up must carry `cmd` CLEAR. Before/after (`macos.rs`):

```rust
// before: its own tracker
fn emit(&self, key: Key, down: bool) {
    self.flags.set(next_flags(self.flags.get(), key, down));
    // ...
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | self.flags.get());
}

// after: held is the source; own bit forced to the press
fn emit(&self, key: Key, down: bool, held: ModifierFlags) {
    // ...
    let untouched = event.get_flags() & !MODIFIERS;
    event.set_flags(untouched | to_cg(flags_for(held, key, down)));
}

/// `held`, with `key`'s own modifier bit set to `down`. Mirrors the old `next_flags`, but with
/// physical `held` as the base rather than a reconstructed tracker.
fn flags_for(held: ModifierFlags, key: Key, down: bool) -> ModifierFlags {
    let mut f = held;
    if let Some(bit) = modifier_bit(key) { f.set(bit, down); }
    f
}
```

`ModifierFlags` is a portable bitset in `freddie_keys` (`CONTROL`/`COMMAND`/`ALT`/`SHIFT`/`CAPS_LOCK`). `to_cg` maps it to `CGEventFlags` and `modifier_bit` maps a key to its `ModifierFlags` bit — both one place, built from the existing per-key `flag_of`. `self.flags`, `next_flags`, and the `Cell` on the emitter are deleted.
