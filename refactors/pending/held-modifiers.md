# one held-modifiers struct, on the root

Not built. There is exactly one record of which modifier keys are physically down, it lives on the root, and it is the single source of truth. Not one per layer (typing's `SetOfHeldKeys`, the paused arm's `HeldModifiers`), and not a second copy in the emitter's flag reconstruction. One struct, one owner.

## The struct

A bool per modifier key, all of them, left and right distinguished:

```rust
#[derive(Debug, Default)]
pub struct HeldModifiers {
    pub meta_left: bool,
    pub meta_right: bool,
    pub control_left: bool,
    pub control_right: bool,
    pub alt_left: bool,
    pub alt_right: bool,
    pub shift_left: bool,
    pub shift_right: bool,
}
```

It lives on `Mercury` (`pub held: HeldModifiers`). It is updated as modifier keys go down and up, in one place, wherever the root sees them (see `root-passthrough.md` for how modifiers route to the root rather than through a layer).

## Why one, and on the root

What is held is a global fact about the keyboard, not a per-layer one. Two copies (typing tracks `cmd`, paused tracks `cmd`/`alt`) means two things that can disagree, and a layer switch that loses the state (a modifier pressed in typing and released after leaving it is invisible to the new layer). One struct on the root cannot drift and does not reset on a layer change.

## What it fixes: stuck modifiers

The single source of truth is what makes the stuck-modifier hazard fixable. The hazard: a `cmd` is passed through (its down reaches the app), then a transition swallows the matching `cmd` up (the unpause chord consumes it, or leaving typing does), so the app is left believing `cmd` is still down.

With `held` on the root, the model always knows `cmd` is really held. So any point that would strand a modifier can consult `held` and emit the corrective release instead of losing it: on leaving a passthrough state, or on consuming a chord, walk `held` and emit an up for every modifier still marked down that will no longer be delivered. The fix is only possible because there is one authoritative record; with the state split across layers, the handler doing the swallow would not reliably know what to close.

## Reconciling the emitter

The emitter keeps its own modifier picture today (`self.flags` / `next_flags` in `freddie_keyboard`), rebuilt from the modifier keydowns it has emitted, so it can stamp the right flags on synthesized chords (`cmd`-`r`, the tmux `ctrl`-`a`). That is a second modifier tracker. With `held` authoritative in the model, the emitter's flags could be derived from `held` rather than reconstructed independently, so there is one source instead of two that can diverge. Whether to unify them, or leave the emitter's as a private detail of output, is the open part.

## Open questions

- Whether the emitter's `self.flags` folds into `held` or stays separate.
- The exact update site: one handler at the root that sees every modifier (depends on `root-passthrough.md` routing modifiers to the root, and on the `no-clobber.md` one-handler-per-event decision).
- A `u128` bitset over every key as the tighter representation, once `held` is the one owner.
- Whether `caps_lock` and `fn` join the struct.
