# enable/disable mercury

Not built. A global off switch: when mercury is off it stops remapping and every key passes through untouched, and one binding (or the menu bar) turns it back on. This is the escape hatch to a normal keyboard without quitting the process, and it is the first mercury feature that needs a laserbeam capability we have only sketched.

## The state is a bool; the hard part is gating the layer on it

The state is trivial: an `enabled: bool` at the root.

```rust
pub struct Mercury {
    pub enabled: bool,
    pub foregrounded: ForegroundedApp,
    pub has_navigated: bool,
    #[resolve_into]
    pub layer: Layer,
}
```

The question is what `enabled` DOES. The whole layer tree should be live when enabled and gone when disabled, so `enabled` has to gate whether dispatch descends into `layer` at all. Today `#[resolve_into] layer: Layer` is unconditional: the root always descends into the active layer. There is no way to say "descend only when `enabled`."

## Why the layer cannot be a derived child

The obvious move is to make the top level a derived child that returns `Option<Layer>`: `Some` when enabled, `None` when not, and `None` means no child, so the root's own binds run instead.

That does not work, and the reason is the point of this doc. Derived children are READ-ONLY. The builder is `fn(&Parent) -> Option<Data>`: it takes a SHARED reference and returns OWNED data, built fresh every dispatch. `app_data` is the example, and its own comment says it: "a shared reference, so it cannot mutate: it derives, it does not act." A derived `Option<Layer>` would hand handlers a COPY of the layer, so a layer transition (`Layer::Home` to `Layer::Nav`, which mutates the real `layer`) would mutate the copy and vanish. Derived is for computed, throwaway levels like Chrome-from-`foregrounded`, precisely because they hold nothing that must persist. The layer is the opposite: it is the persisted state the transitions edit.

So the top-level gate cannot be a derived child. It has to be a projection into the real `layer`, one that is allowed to be absent.

## What it needs: a mutable, state-selected, optional child

The capability is a projection

```rust
fn(&mut Mercury) -> Option<&mut Layer>
```

that reads `enabled` and hands out a mutable reference to the real, persisted layer when enabled, and nothing when disabled. Three properties, and today's two descent kinds each miss one:

- Mutable, so transitions persist. (A derived child is not.)
- Selected by ordinary state, `enabled`, not by matching a type tag. (An enum variant descent is not; it matches the variant.)
- Optional, so "disabled" is a real "no child here" state. (A plain `#[resolve_into]` field is not; it is always present.)

This is exactly `laserbeam-state-controlled-children.md`'s `#[custom_resolve_into_fn]` in its full-projection form, `fn(&mut Node) -> Option<&mut Child>`, together with its fallible-resolve semantics: when the projection returns `None`, resolve stops and the node is itself the active leaf. `enabled` is the discriminator that doc names as "ordinary state, not a type tag." (`option-resolve-into.md` is the neighboring special case, where the child field is itself an `Option`; here the gate is a separate bool, so the custom-fn form is the fit.)

So enable/disable is not a standalone feature to bolt on. It is the concrete first consumer that justifies building the state-selected mutable child, and it is worth writing down because it turns that laserbeam design from speculative into load-bearing for a launch feature.

## Disabled: the root is the leaf

When the projection returns `None`, resolve lands on the root, so the root's own binds are the whole active set. That is where the off-state behavior lives:

- An `AnyKey` passthrough, so every key flows through untouched while disabled, the way typing passes keys through.
- A re-enable trigger that flips `enabled = true`. The next dispatch resolves into the layer again.

Enable/disable is also a natural menu-bar item, a checkbox, the same event-source shape as Quit: an `Enable`/`Disable` event bound at the root, alongside the keyboard toggle. So the switch has two sources, a chord and the menu bar, and the menu-bar one keeps working no matter what the keyboard is doing.

## Open questions

- Disabled in the model versus disabled at the tap. Passing keys through IN the model still intercepts every key (the tap runs, the model re-emits), which is what keeps a keyboard re-enable chord working. Stopping the `CGEventTap` entirely is zero-overhead true passthrough but means no key events at all, so re-enable can then only come from the menu bar. Leaning toward model-passthrough so the chord survives; revisit if the tap overhead while disabled matters.
- The re-enable chord: which keys, and whether it must be a chord unlikely to be typed by accident (disabled means the user is typing normally).
- Whether disabling should release any held modifiers first, the same stuck-key concern the typing `cmd`-`escape` fix handles, so a disable mid-chord does not strand a modifier down.
- How much of `#[custom_resolve_into_fn]` to build: the full state-controlled-children feature, or only the `Option<&mut Child>` projection this needs, with the collection/index forms deferred until something wants them.
