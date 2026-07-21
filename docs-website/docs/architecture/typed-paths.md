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

Each node names its parent, and the derive turns that into the node's path type:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
pub struct NavLayer {
    pub(crate) home_timeout: TimerGuard,
}
```

`#[derive(Bind)]` emits `impl Place for NavLayer`, whose `Path<'a>` is `PathMut<NavLayer, LayerPath<'a>>`. The root says `#[node(root)]` instead, and its path is `&'a mut Mercury`, because there is nothing above it to project out of. The aliases chain by hand in the state module, one line per level:

```rust
pub type MercuryPath<'a> = &'a mut Mercury;
pub type LayerPath<'a> = PathMut<Layer, MercuryPath<'a>>;
pub type AppLayerPath<'a> = PathMut<AppLayer, LayerPath<'a>>;
```

A path type therefore spells out the whole route from the root, and a handler's parameter type says where in the tree the binding sits.

A `PathMut` is not a reference to the node it addresses. It owns the parent and a pair of projections, one `&mut Parent -> &mut Node` and one `&Parent -> &Node`, and re-derives the node each time you call `get_mut` or `get`. The parent's own `dispatch` builds it on the way down with `PathMut::from_fn`: a `#[resolve_into]` field projects to that field, an enum projects through whichever variant is active.

Re-deriving is what keeps the borrows honest. `get_mut` borrows the whole path, so exactly one `&mut` is live at a time, and `into_parent` consumes the path, so the leaf cannot be held across the climb. `laserbeam` pins both with doctests that are required not to compile.
