# enable/disable mercury

Not built. A global off switch: when mercury is off it stops remapping and every key passes through untouched, and one binding (or the menu bar) turns it back on. This is the escape hatch to a normal keyboard without quitting the process.

Disabling must NOT lose where you were. If you are three keys into a nav and you disable, then re-enable, you should be back in that nav, not reset to home. So the layer state has to survive the toggle. That constraint is what picks the design.

## The design: an enable/disable enum that carries the layer in both arms

This is the workaround that ships now. The real fix is `custom-resolve-into.md` (an `enabled: bool` and a `resolve_into` that declines to descend when disabled); the enum below trades that cleaner shape for needing no laserbeam change, and can be deleted for the real fix later.

Wrap the layer in a two-variant enum, and give BOTH variants the layer state. It is a bit weird, but it unblocks the feature with no laserbeam change:

```rust
pub struct Mercury {
    pub foregrounded: ForegroundedApp,
    pub has_navigated: bool,
    #[resolve_into]
    pub power: Power,
}

// Enable/disable is the enum discriminant, NOT a separate `enabled: bool` next to a
// single `layer` field. A bool would need laserbeam to descend into `layer` only
// when enabled, and `#[resolve_into]` is unconditional -- there is no "descend only
// if" today. Making the toggle the variant instead rides the enum-variant descent
// laserbeam already has (mutable, so transitions persist), and carrying the layer in
// BOTH arms is what preserves the layer state across a toggle: enabling and disabling
// MOVE the layer between arms, they do not reset it. A bool beside the layer would
// have kept the state too, but could not gate the descent; a derived `Option<Layer>`
// could gate but is read-only (owned copy), so transitions would vanish. The enum is
// the shape that gets both.
pub enum Power {
    Enabled(Enabled),
    Disabled(Disabled),
}

pub struct Enabled {
    #[resolve_into]
    pub layer: Layer, // resolve_into: descended into, so the layer tree is live
}

pub struct Disabled {
    pub layer: Layer, // plain field: held dormant, NOT descended into, while off
}
```

Toggling is a variant transition that MOVES the layer from one arm to the other, exactly like the layer transitions handlers already do (`Layer::Home` to `Layer::Nav`). Enabling does `Power::Enabled(Enabled { layer })` with the `layer` moved out of `Disabled`; disabling does the reverse. Nothing is cloned and nothing is reset, so the layer and everything under it (the active layer, the in-app state) is exactly preserved across the toggle.

The weird part is that `Enabled` and `Disabled` hold the SAME type differently: `Enabled` marks the layer `#[resolve_into]` so the tree is active, `Disabled` holds it as a plain field so the active `Disabled` node is a leaf and the layer's bindings are off. Same data, different descent, which is what makes disabled a real "stop here" without dropping the layer.

The `Power` level adds one hop to the tree, so paths that ascend from inside the layer to the root (for instance `app_data` reading `root.foregrounded`) gain a level: `... -> Layer -> Enabled -> Power -> Mercury`. `foregrounded` stays on `Mercury`, above `Power`, so foreground tracking is unaffected by the toggle.

## Disabled behavior

The active `Disabled` node is a leaf, so its own binds are the whole active set:

- An `AnyKey` passthrough, so every key flows through untouched while off, the way typing passes keys through.
- A re-enable trigger that moves the layer back into `Power::Enabled`. The next dispatch descends into the layer again, right where it left off.

## The menu bar

Enable/disable is also a natural menu-bar item, a checkbox, the same event-source shape as Quit: an `Enable`/`Disable` event that flips `Power` at the root, alongside the keyboard toggle. So the switch has two sources, a chord and the menu bar, and the menu-bar one keeps working no matter what the keyboard is doing.

This side is downstream of making the menu bar reflect state (the "later" half of `menu-bar.md`, the `label`/`menu` derived from the state tree). The toggle wants a checked menu item that follows `Power`, and ideally the icon to show disabled at a glance. So the keyboard side of enable/disable can land on its own, but the menu-bar side rides on the state-reflecting menu-bar content and sequences after it.

## Open questions

- Disabled in the model versus disabled at the tap. Passing keys through IN the model still intercepts every key (the tap runs, the model re-emits), which keeps a keyboard re-enable chord working. Stopping the `CGEventTap` entirely is zero-overhead true passthrough but means no key events at all, so re-enable then comes only from the menu bar. Leaning toward model-passthrough so the chord survives; revisit if the tap overhead while disabled matters.
- The re-enable chord: which keys, and whether it must be unlikely to be typed by accident (disabled means the user is typing normally).
- Whether disabling should release any held modifiers first, the same stuck-key concern the typing `cmd`-`escape` fix handles, so a disable mid-chord does not strand a modifier down.
- Whether the `Enabled`/`Disabled` wrapper structs are the cleanest shape, or whether laserbeam should instead grow per-variant control over whether a variant descends into its payload, which would let the arms be `Enabled(Layer)`/`Disabled(Layer)` directly without the wrappers.
