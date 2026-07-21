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

`state.handle` takes state and event and returns the updated state and the effects. It performs none of them, so a test is a function call and an `assert_eq!`. There is no keyboard to drive, no daemon to start, and nothing to mock.

```rust
#[test]
fn home_n_enters_nav() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        Some(vec![shows("Nav"), return_home_timer()])
    );
    assert!(matches!(m.layer(), Layer::Nav(_)));
}
```

Three things are asserted, and the third is the one people forget:

- The event was handled at all. `handle` returns `Option`, and `None` means no binding claimed it.
- The exact effects, in order. Not that a `ShowLayer` appeared somewhere, but that these two came back and nothing else did.
- The resulting state. `n` from home leaves you in nav, and a binding that produced the right effects while landing in the wrong layer is still wrong.

### Where a test starts

Tests build the state they mean to exercise rather than pressing keys to get there:

```rust
// A mercury in Home, the command layer. The default is Typing (passthrough), but most
// per-event tests exercise Home's command bindings, so they start here.
fn home() -> Mercury {
    Mercury::with_layer(Layer::Home(HomeLayer {}))
}
```

A test that walks in from the typing layer is testing the walk as well as the binding, and fails for two reasons instead of one.

### Timers

A timer effect mints an id, so a test cannot write the id it expects. It rebuilds the effect and compares:

```rust
fn return_home_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, fired);
    MercuryEffect::Timer(effect)
}
```

Under the `testing` feature, equality on a timer effect compares the delay and the event it will fire, not the id. To assert on a firing, read the id back off the effect that set it, since nothing else can know it.

### The standard

The standard for the model is exhaustive: every key in every reachable state, asserting exactly what dispatch produces. Because the model is a pure function of state and event, the full table is checkable, and it doubles as documentation of the keymap. `crates/mercury/tests/transitions.rs` holds 87 of these today, which is not the whole table. A new binding extends toward it rather than testing only the happy path.

### Driving the loop

The per-event tests above call `handle` directly. When a test needs several events to feed each other, it drives a `bind::SimpleRunner`, which records the effects and, for a `Foreground` effect, reports the app back the way the OS watcher would.

## Where the binding leaves you

TODO: `and_go_home`, `to_typing`, and staying in the layer, and how to pick.
