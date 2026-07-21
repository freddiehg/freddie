---
title: The Data Model
sidebar_position: 3
---

# The Data Model

The data model is what controls which handler is executed when you call `state.handle(event)`. `mercury` intentionally has a fairly standard one, designed to be extended and modified for your use case.

In the simplest case, the state is a nested enum. `struct Mercury` contains a `#[resolve_into] layer: Layer` field, which is an enum. Different keys can be bound on different layers: `c` navigates to Google Chrome iff `matches!(state.layer, Layer::Nav(_))`, but not in other layers.

## Making impossible states unrepresentable

The standard is that a state which cannot occur should not be writable. A field that is meaningless when another field takes some value does not sit beside that field; it sits inside the variant that gives it meaning. Where holding to that costs a refactor, the refactor is the cheaper of the two.

The layer enum is that rule applied to modes. `mercury` is in exactly one layer, and while it is, only that layer's state exists. `NavLayer` holds the return-home `TimerGuard` it armed, and the moment the layer changes, that value is gone. There is no "which layer" tag sitting beside a bag of per-layer fields, so no tag and no field can disagree, and one layer's state cannot be read while another is active.

The same shape appears below the layers. `ForegroundedChrome`, which holds the front tab's URL, exists only inside one variant:

```rust
pub enum ForegroundedApp {
    Chrome(ForegroundedChrome),
    Finder,
    Ghostty,
    Zed,
    Other,
}
```

So there is no URL to go stale while Finder is up, and nothing to clear when Chrome goes away: the value leaves with it. The overlay is the one-variant version of the same move, an `Option<TimerGuard>` at the root rather than a showing flag beside a timer that may or may not mean anything.

Dispatch is where the discipline pays off. A handler bound on `NavLayer` is handed a path to a `NavLayer`, so it never asks which layer it is in and never writes an arm that cannot be reached. See [Typed Paths](./typed-paths.md).

## Where state lives

State lives on the level that uses it. The return-home timer is the clearest case: the nav, in-app and site layers each arm one when they are entered, and each holds its own guard.

```rust
#[node(parent = LayerPath)]
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
)]
pub struct NavLayer {
    pub(crate) home_timeout: TimerGuard,
}
```

Two things follow from the guard living there and nowhere else. Leaving the layer drops the `NavLayer`, which drops the guard, which cancels the timer, so no transition has to remember to cancel anything. And the trigger is read out of the guard the layer still holds, so a firing from a layer you have already left matches nothing and reaches no handler.

Put that one guard at the root instead and both properties go. The timer outlives the transition, so a firing armed by nav can arrive while you are somewhere else, and the model then owes an answer to which layer a firing belongs to, plus a cancel on every transition that could forget one layer. Both are answers to a question the data model can simply stop asking.

The rule cuts the other way as often. `TypingState` sits at the root, because entering and leaving a passthrough layer reads `held` to synchronize the app's view of the modifier keys, so it has to outlive any one layer. Its `jk` sequence sits beside it and is replaced with a fresh `KeySequence` on every layer change, so a half-typed run never crosses a boundary. The test is which level uses the state, not which level is convenient to hang it on.
