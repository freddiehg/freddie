---
title: Typed Paths
sidebar_position: 5
---

# Typed Paths

`laserbeam` is the typed mutable path that bindings are built over. A handler bound on a level is handed a path to that level, so it can call `get_mut` unconditionally.

```rust
fn louder<'a>(_ev: &KeyEvent, node: Node<VolumeLayerPath<'a>, ()>) -> MercuryEffect {
    let layer: &mut VolumeLayer = node.parent.get_mut();
    layer.volume = layer.volume + 10;
    MercuryEffect::SetVolume(layer.volume)
}
```

Written against the whole state instead, the handler has to recover what dispatch already knew:

```rust
fn louder(state: &mut Mercury) -> Vec<MercuryEffect> {
    let Layer::Volume(layer) = &mut state.layer else {
        unreachable!("bound in the volume layer")
    };
    // ...
}
```

That `unreachable!` has nothing to guard, and the compiler cannot tell. A state a binding cannot be reached in is not an arm that panics, it is a value the handler is never handed.

## Climbing

A handler that needs more than its own level climbs. `node.parent.into_parent()` is the `Layer` above, and one more is the root, `&mut Mercury`, which is how `esc` sets the layer back to home from wherever it was pressed.

## How the path is built

TODO: what the derive generates, and how a `Path` type relates to the struct it addresses.
