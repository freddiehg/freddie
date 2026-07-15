# passthrough as a count at the root

Not built. A cleanup of how mercury decides to pass keys through, and where held-modifier state lives.

## The problem

Two things are scattered that are really one fact each.

Held modifiers are tracked in two places: typing's `SetOfHeldKeys { cmd }` and the paused arm's `HeldModifiers { cmd, alt }`. What is physically held is one global fact, but each layer that cares keeps its own copy, because with one-handler-per-event dispatch a root-level modifier handler never fires when a layer also handles the key (see `no-clobber.md`). So the natural home for modifier state, the root, is the one place that cannot currently see every key.

"Pass this key through untouched" is re-implemented per layer: typing's `AnyKey` catch-all emits, and the paused arm's `AnyKey` catch-all emits. Whether mercury is in passthrough is one global fact too, but it lives in whichever layers happen to want it.

## The design: one passthrough count at the root

`Mercury` gains a passthrough count, a small unsigned integer, next to the held-modifier state (two separate fields, not one object). Passthrough is on while the count is greater than zero.

Two states want passthrough: the typing layer, and the paused state (the typing layer only exists while unpaused; paused is its own state). Each holds a guard that increments the count when it comes into existence and decrements it when it is dropped. Entering typing bumps the count; leaving typing drops the `TypingLayer` and un-bumps it; pausing bumps it; unpausing drops the paused state and un-bumps it.

It is a count, not a bool, because the two sources overlap and drop in an order we do not control. Pausing while the layer underneath is typing has both guards live at once; a bool would be cleared by whichever drops first even though the other still wants passthrough. A count is order-independent: passthrough is on while any source is live, off only when the last one drops.

## Construction discipline: only through a method

`TypingLayer` and the paused state must not be constructed directly, no struct literal. They are built by a method that wires the guard, so the increment cannot be forgotten, and their `Drop` does the decrement. A direct construction would make one without a guard and desync the count.

So entering typing goes through a `Layer` method rather than assigning `Layer::Typing(TypingLayer { .. })`, and pausing goes through a `Power` method. Entering a layer is already method-shaped (the transition handlers), so this is where the guard is created; the raw variant construction is what goes away.

## Modifiers move to the root

No layer binds `cmd`, `alt`, or `opt`. The root owns the held-modifier state and the modifier keys' handlers, and matches on the active layer where the behavior needs to differ.

Layers bind only their own commands. A key a layer does not bind falls through to the root; when the passthrough count is greater than zero the root emits it, otherwise it is swallowed (command mode). This replaces the per-layer emit-everything catch-alls (typing's `AnyKey`, the paused arm's `AnyKey`). The paused arm stops needing `HeldModifiers`, and typing stops needing `SetOfHeldKeys`.

## The mechanism to settle

The guard has to reach the root's count from its `Drop`, which a plain field in a value tree does not give it. The straightforward way is a shared cell: `passthrough: Rc<Cell<u32>>` on the root, each guard holding a clone, incrementing on construction and decrementing on drop. That puts shared mutable state into a tree that is otherwise pure values, which is the cost to weigh. The alternative is explicit increment/decrement in the transition methods without `Drop`, which keeps the tree pure but loses the "cannot forget" that `Drop` buys.

## How today's features land on it

- Typing passthrough: the typing guard keeps the count up; typing's escape and cmd-escape stay as its own binds; its `AnyKey` catch-all is replaced by fall-through to the root's passthrough.
- Paused passthrough: the paused guard keeps the count up; the layer underneath is not descended into, so the root's passthrough is what emits keys.
- The `cmd`-`alt`-`p` unpause chord: the root holds the modifier state now, so it recognizes the chord while paused and unpauses (dropping the paused guard). The chord logic moves to the root with everything else.

## Open questions

- The shared-cell mechanism versus explicit increment/decrement, and whether keeping the state tree pure values is worth giving up `Drop`'s safety.
- Which modifiers the root tracks (cmd, alt, opt, ctrl, shift) and which of them passthrough actually cares about.
- How the root matches on the active layer for per-layer modifier behavior under one-handler-per-event, and whether this is the case that finally forces the `no-clobber.md` decision (letting the root always see modifiers, instead of the fall-through dance).
- The integer width (`u8` is plenty for two sources; it does not matter much).
