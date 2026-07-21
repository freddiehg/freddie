---
title: Implementing Your Own Handler
sidebar_position: 3
---

# Implementing Your Own Handler

A binding is a trigger and the handler it runs, written on the level where it applies.

## The simplest one there is

`mercury`'s nav layer foregrounds apps. `c` goes to Chrome, `g` goes to Ghostty. The bindings are a list on the layer they belong to:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyG.down() => open_ghostty,
)]
pub struct NavLayer {}
```

And a handler is a function:

```rust
fn open_chrome<'a>(
    _ev: &KeyEvent,
    _node: Node<NavLayerPath<'a>, ()>,
) -> MercuryEffect {
    MercuryEffect::Foreground(App::Chrome)
}
```

That is the whole thing. It reads neither the event nor the state, because it does not need to: it only runs when `c` went down in the nav layer, and dispatch already established both of those before calling it.

## One that uses state

Handlers that need state are handed a path to the level they were bound on. Say the resize layer, where `up` maximizes the focused window and `r` puts it back where it was. Where it was has to be remembered, so it lives on the root:

```rust
pub struct Mercury {
    /// The focused window and where it sits.
    focused: Option<(WindowId, Frame)>,
    /// Where each window was before we moved it.
    prior_locations: HashMap<WindowId, Frame>,

    #[resolve_into]
    layer: Layer,
}
```

Maximizing writes it down:

```rust
fn maximize<'a>(
    _ev: &KeyEvent,
    node: Node<ResizeLayerPath<'a>, ()>,
) -> Option<MercuryEffect> {
    let root: &mut Mercury = node.parent.ascend();

    let (id, frame) = root.focused?;
    // Only the first maximize records anything. A second one
    // finds the entry already there and leaves it alone, so
    // `r` still goes back to where the window started.
    root.prior_locations.entry(id).or_insert(frame);

    Some(MercuryEffect::Place(Placement::Maximize))
}
```

And restoring reads it back:

```rust
fn restore<'a>(
    _ev: &KeyEvent,
    node: Node<ResizeLayerPath<'a>, ()>,
) -> Option<MercuryEffect> {
    let root: &mut Mercury = node.parent.ascend();

    let (id, _) = root.focused?;
    let frame = root.prior_locations.remove(&id)?;

    Some(MercuryEffect::Place(Placement::Exactly(frame)))
}
```

`node.parent` is the path to the level the binding was written on, and `ascend` climbs from it to the root. There is no checking whether resize is the active layer, and no `unreachable!` for the case where it is not. `restore` runs because it was, and the path is what says so. A state a binding cannot be reached in is not an arm that panics, it is a value the handler is never handed.

`remove` rather than a lookup, because restoring forgets: press `r` twice and the second press does nothing rather than placing the window again.

## What a handler returns

Return whatever suits the handler. `open_chrome` has exactly one effect, so it returns one. `restore` either has one or has none, so it returns an `Option`. A handler with several returns a `Vec`.

```rust
fn open_chrome(..) -> MercuryEffect          // always one
fn restore(..) -> Option<MercuryEffect>      // one or none
fn quit(..) -> Vec<MercuryEffect>            // several
```

Dispatch converts whatever comes back into the one type it hands the effect loop. Which return types are legal is a decision `mercury` makes rather than one freddie imposes, so if you want a new one you add it.

## Choosing the level to bind on

Bind on the deepest level where the binding is true.

Levels nest, and dispatch walks them from the deepest outwards, taking the first handler whose trigger matches. So `r` bound on `ResizeLayer` beats `r` bound on `Mercury`, and the root's binding never runs while resize is active.

That ordering is what lets a level be specific without every other level knowing about it:

- A key that only means something in one layer goes on that layer. `c` opening Chrome makes no sense outside nav, so it lives on `NavLayer` and nothing else has to exclude it.
- A key that means the same thing everywhere goes on the root. `esc` returning home is bound once, and every layer inherits it by not overriding it.
- A key that means one thing in most places and something else in one place is bound on both. The specific level wins wherever it applies, and the general one covers the rest.

The root is also where the last resort goes. `AnyKey` matches every key event there is, and sits at the root so that anything no layer claimed still has somewhere to land: in the typing layer, that is what passes your keystrokes through.

## What happens next

Every binding decides which layer it ends in, and the decision follows from what you are expected to do next.

- Something you would plausibly do again right away stays put. Walking tmux's windows and refreshing a page both repeat, so they stay in the layer.
- Something that is a choice rather than a repetition leaves. Placing a window is one decision, so it goes home.
- Anything followed by typing lands in the typing layer. Chrome's `l` focuses the address bar, so it ends there. A command layer would have swallowed whatever you typed next.
