---
title: What Is a Separate Handler
---

# What Is a Separate Handler

The schedule makes it cheap to attach several handlers to one trigger, so the question stops being "can I split this" and becomes "what deserves to be its own handler." Four things do, and only four.

## One user gesture, whole

An exclusive bind owns the whole of one user action: every state write and every effect the gesture implies, in one claimed slot. Foregrounding Chrome without entering the in-app layer is not a behavior; `Kill` without opening the held modifiers is a bug. So a gesture is never split across schedule slots — that is the same mistake as two gestures in one handler, mirrored. When a gesture has parts, they compose inside its one bind:

```rust
#[bind(Key::KeyC.down() => and!(mark_navigating, foreground_chrome, enter_inapp))]
```

`and!` runs the units in order under a single claim, each seeing the state the previous one left. The units are reusable; the gesture is the bind.

## One cross-cutting concern

A `#[post]` or `#[pre_post]` owns one job that cuts across gestures, keyed on its trigger and on whether the descent stayed or left. It never claims, so it runs whether or not a gesture handled the key — which is the point. The return-home deadline is one post on the node that owns the timer: on a stay it overwrites the guard with a freshly armed one, and the overwrite is the cancel, because a dropped guard cancels through freddie's cancel channel; on a leave the layer swap already dropped the guard, so it does nothing. Held-modifier tracking is one root post, because a modifier pressed inside a layer must be tracked even though the layer claimed the key.

```rust
#[post(AnyKey => home_deadline)]
#[post(AnyKey => track_held_modifiers)]
```

A concern folded into every gesture that could affect it is the smell this replaces; a concern spread across `handle` after dispatch is the same smell in one place.

## One recorded fact

A recorder is an exclusive bind at the root that assigns one field from an event reporting external truth, and stays: the front app, the front tab's URL, the window list. Assignment, never accumulation, so replaying the event lands in the same state.

## One timer consequence

A timer bind matches one timer id through a state-reading trigger and applies its one consequence. The trigger is an `Option`: no armed timer, no match.

```rust
#[bind(|m| m.overlay_timer().map(TimerGuard::trigger) => hide_overlay)]
```

## What is not a separate handler

- **A mutation method's implied effects.** `set_layer` hides the overlay, resets jk, opens or closes modifiers, and shows the new layer; those are what the one state write implies, so they belong to the method, not to scheduled items beside it.
- **A gesture's steps.** Units composed by `and!` share one bind and one claim; they are not schedule slots.
- **A `TimerGuard`'s `Drop`.** Dropping a guard is the cancel — it cancels through freddie's cancel channel — so the rearm's overwrite and a layer swap both cancel by dropping, and no handler carries a cancel step.
- **A derived child fn.** `app_data` and its kind are resolve inputs that build a level from state; they decide nothing and emit nothing.

## Where a handler ends

What a handler touches decides where it ends, and the ending must be truthful, because posts key decisions on it: `Invalidated` downstream must always mean the focus genuinely moved.

- Its own node: mutate through the standing path and complete there.
- Effects only: complete wherever the state stands; any branch is fine.
- The root: consume to it and complete there — legitimate exactly when the gesture ends at the root anyway, as every `set_layer` does.
- State written from many layers: bind the key at the node that owns the field, so the write is an own-node write. The overlay's `o` binds at the root, which owns `overlay`, rather than five times on the layers below it.

A handler that ends somewhere it does not mean to be is lying to every post scheduled after it.
