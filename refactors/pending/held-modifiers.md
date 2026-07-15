# one held-modifiers struct, on the root

Not built. The minimal fix for modifier passthrough: one record of which modifier keys are physically down, on the root, single source of truth (not one per layer -- typing's `SetOfHeldKeys`, the paused arm's `HeldModifiers` -- and not a second copy in the emitter). It does two things and no more: hold the state, and gate passthrough on it -- a modifier is emitted through to the app IFF a passthrough layer is active, and swallowed otherwise. Nothing here about routing, `NonModifierKey`, or a root catch-all; those are separate docs and not in scope.

## The struct

One `LeftRightPair` per modifier, so left and right stay distinct but the caller asks about a modifier ("is `control` held") without spelling out both sides:

```rust
#[derive(Debug, Default)]
pub struct LeftRightPair {
    pub left: bool,
    pub right: bool,
}

impl LeftRightPair {
    pub fn any_held(&self) -> bool { self.left || self.right }

    pub fn hold_left(&mut self)     { self.left = true; }
    pub fn hold_right(&mut self)    { self.right = true; }
    pub fn release_left(&mut self)  { self.left = false; }
    pub fn release_right(&mut self) { self.right = false; }
}

#[derive(Debug, Default)]
pub struct HeldModifiers {
    pub control: LeftRightPair,
    pub meta: LeftRightPair,
    pub alt: LeftRightPair,
    pub shift: LeftRightPair,
}
```

`Default` gives all-false. It lives on `Mercury` (`pub held: HeldModifiers`): `held.control.hold_left()` on a `ControlLeft` down, `held.control.release_left()` on its up, `held.control.any_held()` to ask.

## Passing modifiers through, iff a passthrough layer is active

A modifier reaches the app only where keys pass through: in a passthrough layer (typing, or paused). Everywhere else it is swallowed like any other unbound key. That gate already exists in the current architecture -- typing and paused have a catch-all that emits the key, and command layers (home, nav, in-app) have none, so an unbound modifier there is dropped by `decide`. So this is not new routing; it is the same catch-alls, now also recording into `root.held` as they pass a modifier through:

- passthrough layer, modifier down: `hold_*` the side, then emit it (pass through).
- passthrough layer, modifier up: emit it, then `release_*` the side.
- command layer: no catch-all, so the modifier is swallowed and `held` is not touched -- nothing passed through, nothing to strand.

Moving the catch-all itself to the root, so one handler does this for every layer, is the follow-up (`root-passthrough.md`). This doc leaves the handlers where they are and only moves the state.

## Why one, and on the root

What is held is a global fact about the keyboard, not a per-layer one. Two copies (typing tracks `cmd`, paused tracks `cmd`/`alt`) means two things that can disagree, and a layer switch that loses the state (a modifier pressed in typing and released after leaving it is invisible to the new layer). One struct on the root cannot drift and does not reset on a layer change.

## What it fixes: stuck modifiers (necessary, not sufficient)

The hazard: a `cmd` is passed through (its down reaches the app), then a transition swallows the matching `cmd` up (the unpause chord consumes it, or leaving typing does), so the app is left believing `cmd` is still down.

One authoritative `held` is a PRECONDITION for the fix, not the fix. It lets the model know `cmd` is really down; with the state split across layers the swallowing handler would not reliably know what to close. But knowing is not closing. The actual fix is the corrective-emit wiring: at every point that swallows a modifier's up -- leaving a passthrough state, consuming a chord -- walk `held`, emit an up for each side still marked down, and `release_*` it. That wiring is the work; `held` only makes it expressible. Where exactly those release points are, and whether one shared helper covers all of them, is still open below.

## Reconciling the emitter

The emitter keeps its own modifier picture today (`self.flags` / `next_flags` in `freddie_keyboard`), rebuilt from the modifier keydowns it has emitted, so it can stamp the right flags on synthesized chords (`cmd`-`r`, the tmux `ctrl`-`a`). That is a second modifier tracker. With `held` authoritative in the model, the emitter's flags could be derived from `held` rather than reconstructed independently, so there is one source instead of two that can diverge. Whether to unify them, or leave the emitter's as a private detail of output, is the open part.

## What we're going to implement

- `LeftRightPair` and `HeldModifiers` (four pairs, `Default`) as above, on `Mercury`.
- The passthrough-layer catch-alls (typing, paused) `hold_*` / `release_*` into `root.held` as they pass a modifier through; command layers are unchanged.
- A corrective-emit helper: given `held`, produce an up for every side still down, for a transition to call.
- Wire that helper into the two swallow points that exist today (leaving typing, the unpause chord).

## Open questions

- The corrective-emit wiring: the exact set of release points, and whether one shared helper (walk `held`, emit every held modifier's up) covers both transitions or each needs its own.
- Whether the emitter's `self.flags` folds into `held` or stays separate.
- A `u128` bitset over every key as the tighter representation, once `held` is the one owner (harder to reconcile with the `LeftRightPair` API; the two are in tension).
- Whether `caps_lock` and `fn` join the struct.
