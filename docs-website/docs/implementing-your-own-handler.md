---
title: Implementing Your Own Handler
sidebar_position: 3
---

# Implementing Your Own Handler

A binding is a trigger and the handler it runs, written on the level where it applies.

## A worked example

Say we want a volume layer, where `up` and `down` change the volume and the layer remembers what it set it to. The volume lives on the layer, because that is the only place it is used:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => louder,
    Key::DownArrow.down() => quieter,
)]
pub struct VolumeLayer {
    volume: u8,
}
```

And the handler:

```rust
fn louder<'a>(_ev: &KeyEvent, node: Node<VolumeLayerPath<'a>, ()>) -> MercuryEffect {
    let layer: &mut VolumeLayer = node.parent.get_mut();
    layer.volume = layer.volume + 10;
    MercuryEffect::SetVolume(layer.volume)
}
```

`node.parent` is the path to the level the binding was written on, so `get_mut` hands back this layer, unconditionally. There is no question of whether the volume layer is the active one: `louder` runs because it was, and the path is what says so.

## What a handler returns

`louder` asks for one thing, so it returns one thing. `Bindings::Output` is what dispatch returns, and for `mercury` that is still `Vec<MercuryEffect>`. A handler returns anything that is `Into` the output, and dispatch converts it.

```rust
/// One effect, returned bare.
pub(crate) const fn refresh<E, N>(_ev: &E, _node: N) -> MercuryEffect {
    tap(Key::KeyR, ModifierFlags::COMMAND)
}

/// Several, returned as the vector.
pub(crate) fn replay(presses: Vec<KeyPress>) -> Vec<MercuryEffect> {
    // ...
}
```

The conversion is the program's, not the framework's:

```rust
impl From<MercuryEffect> for Vec<MercuryEffect> {
    fn from(effect: MercuryEffect) -> Self {
        vec![effect]
    }
}
```

So the set of things a handler may return is something you extend. Writing `From<Option<MercuryEffect>>` lets a handler decline to produce one, and `From<()>` covers the handlers that only mutate state.

## Climbing to a parent

TODO: `node.parent.into_parent()` reaches the `Layer` above, and one more reaches the root `&mut Mercury`. Show `esc` setting the layer back to home from wherever it was pressed.

## Choosing the level to bind on

TODO: explain how precedence works between a layer's bindings and the root's, and where to put a binding that should apply everywhere.

## Testing a handler

TODO: `state.handle` is a pure state transformer, so a test asserts exactly what a given state and event produce. Show one test and describe the exhaustive-table standard.

## Where the binding leaves you

TODO: `and_go_home`, `to_typing`, and staying in the layer, and how to pick.
