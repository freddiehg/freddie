# remove pause

Prerequisite for `held-modifiers.md`. Roll the pause/unpause feature back out entirely: no `Power` enum, no `Paused`/`Unpaused` arms, no toggle, no menu-bar pause item. Mercury holds the layer directly again, exactly as it did before `enable-disable.md` (now in `refactors/past`).

## Why

Two reasons, both pointing the same way.

The paused arm and the typing layer are now the same thing. Both are "mercury is live but every key passes through untouched." `Paused` does it by not descending into the layer and running an `AnyKey => pass_through` catch-all; typing does it with its own `AnyKey` catch-all. The unpause chord (`cmd`-`alt`-`p`) is the only behavioral difference, and it is a command like any other. So pause is a second, parallel implementation of passthrough that carries its own held-modifier tracking (`Paused::held`), its own catch-all, and an extra `Power` level in the tree.

`held-modifiers.md` moves passthrough to the root and makes "are we passing through" a predicate on the active layer. With pause still present there are TWO passthrough states (typing, and paused-over-any-layer), which is the only thing that forced a passthrough COUNT and the `PassthroughGuard` machinery. Delete pause and there is exactly one passthrough layer (typing), the count collapses to a bool read off the tree, and the guard disappears. Pause is the sole reason the harder design existed.

Pause can come back later as `custom-resolve-into.md` (an `enabled: bool` and a `resolve_into` that declines to descend when disabled), which is the clean shape `enable-disable.md` itself named as the eventual replacement for the enum workaround. This doc removes the workaround; it does not foreclose the feature.

## What comes out

### `state.rs`

The `Power` enum, both arms, and the paused held struct are deleted. `Mercury` holds the layer directly:

Before:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    #[resolve_into]
    pub power: Power,
}

pub enum Power { Unpaused(Unpaused), Paused(Paused) }

pub struct Unpaused {
    #[resolve_into]
    pub layer: Layer,
}

pub struct HeldModifiers { pub cmd: Option<Key>, pub alt: Option<Key> }

#[bind(AnyKey => pass_through)]
pub struct Paused {
    pub layer: Layer,
    pub held: HeldModifiers,
}
```

After:

```rust
pub struct Mercury {
    pub foregrounded: App,
    pub has_navigated: bool,
    #[resolve_into]
    pub layer: Layer,
}
```

`Power::layer`/`layer_mut`/`toggle`/`pause`/`unpause` all go with the enum. `Mercury::layer` reads `&self.layer` directly instead of `self.power.layer()`. `Default` builds `layer: Layer::Home(HomeLayer {})` on `Mercury` directly.

The path aliases lose a level. `PowerPath`, `UnpausedPath`, and `PausedPath` are deleted; `LayerPath`'s parent becomes `MercuryPath`:

```rust
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
```

Everything below the layer keeps its parent alias, so only the `Layer -> ... -> Mercury` chain shortens by one hop.

### The ascents shorten by one

Every path that climbs from inside the layer to the root loses the `Unpaused`/`Power` hop. `app_data` ascends `AppLayer -> Layer -> Mercury` rather than `AppLayer -> Layer -> Unpaused -> Power -> Mercury`:

```rust
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    let root = path.parent().parent().parent(); // one fewer parent()
    // ...
}
```

The layer-transition handlers in `home.rs` (`to_home`, `to_nav`, `to_typing`, `to_inapp`, `to_resize`) ascend to `LayerPath`/`MercuryPath` instead of through `UnpausedPath`/`PowerPath`. They are already generic over the ascend path, so the change is in the alias, not the handler bodies.

### `home.rs`

The `pause` handler is deleted, and `Key::KeyP.down() => pause` comes off `HomeLayer`. Home no longer has a keyboard pause binding.

### `toggle.rs`

The whole file goes: `on_toggle` and the paused arm's `pass_through` (with the unpause chord) both belong to pause. Remove the module from `handlers/mod.rs`.

### `sources.rs` and `lib.rs`

`Toggle`, `ToggleEvent`, and `toggle_event` are deleted, and their re-exports come out of `lib.rs`. The root's `Toggle => on_toggle` binding comes off `Mercury`. `Unpaused` stops being exported.

### The menu bar

`freddie_menu_bar` drops the "Pause / Unpause" item and its `on_toggle` callback, leaving just Quit. The wiring in `main.rs`/`model.rs` that fed toggle events into the model comes out with it.

## What does NOT change

The layer set (`Home`/`Nav`/`Resize`/`Typing`/`InApp`), every layer's own bindings, foreground tracking, and quit are untouched. This is purely the removal of the pause LEVEL and its two parallel passthrough implementations. After it, `held-modifiers.md` builds passthrough once, at the root, over the single remaining passthrough layer.
