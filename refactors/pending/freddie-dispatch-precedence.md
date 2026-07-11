# precedence and dispatch

How multiple bindings on the active path resolve to one behavior. Implemented; this records the model and what is still open. Terminology: leafward means closer to the active leaf (the more-specific node), rootward means closer to the root.

## The model: priority across the whole path

A trigger tested against an event returns `Match::Handle(Priority) | DontHandle`, where `Priority` is an `i32`, higher wins. Dispatch is two passes over the active path:

- `winner` reads the tree and returns the highest priority any active binding gives the event.
- `dispatch_at` runs the binding that handles at exactly that priority, trying the active child first, so among equal priorities the leafward one wins.

So the winner is chosen by priority first and position second. That is the whole point, and it is what a bare `bool` and leaf-wins could not express.

## Why bool was wrong

The old rule was leafward-wins by position: dispatch tried the active child first and took the first match. That is correct until the leaf's binding is a wildcard. mercury's typing layer binds a catch-all that matches every key, and it sits at the leaf, so under leaf-wins it shadowed the layer-level `escape` binding above it. The workaround was to re-bind `escape` in typing too, which is a wart that scales with every catch-all and every key it should not swallow.

The overlap is invisible to equality. `accumulate` still dedups triggers by `Eq`/`Hash`, and `AnyKey` and `KeyPress(escape)` are different values, so it cannot see that one's match set contains the other's. Detecting overlap in general is undecidable (two predicates over key and press). Priority sidesteps detection entirely: the wildcard declares itself lower, and the specific key wins wherever they overlap, with no comparison of match sets.

## How a trigger picks its priority

The trigger's `try_match` returns the priority. In freddie_keys a specific key handles at `SPECIFIC` (0). In mercury the catch-all handles at `WILDCARD` (`SPECIFIC - 1`), one below, so a named key always beats it. A trigger that must never be overridden returns a priority nothing else reaches; a trigger meant as a fallback returns a negative one.

Priority is a property of the trigger, not the binding site. Two bindings using the same trigger type get the same priority, and ties break leafward. If a specific binding must outrank another specific binding regardless of position, it needs a distinct trigger that returns a higher priority. That is the shape of a global escape hatch: a dedicated trigger at a priority above every layer's.

## What this gives, and what it does not

It gives the general answer to "keep this binding from being clobbered": make its priority higher than whatever overlaps it. A wildcard cannot shadow a specific key; a rootward global cannot be shadowed by a leafward wildcard.

It does not, on its own, give a truly unclobberable binding against a deliberate leafward binding of the *same* priority, because that is a tie and leafward wins. That is correct: a layer that explicitly rebinds a key is overriding on purpose. A binding that must survive even that needs its own high-priority trigger, which the model supports but mercury has not yet defined.

It also does not give the killswitch its replacement for free. A global quit still needs a key the model can recognize, and a modifier chord like ctrl-q is not one event the model sees today, because modifier state is not tracked (modifier-keys.md). The precedence is the mechanism; the trigger for a global quit is the missing piece.

## The road not taken

Two alternatives were considered and are worse for this.

Static winner chosen at accumulation. Pick one handler per trigger up front. But a wildcard and a specific key are different triggers, so this still needs overlap detection, which is the thing priority avoids.

Dynamic fall-through, where a matched handler returns whether it handled the event and dispatch falls through on decline. This is strictly more powerful, since a leafward handler can take some cases and pass the rest rootward. It is also more machinery: a chain kept per trigger and a handler-return protocol. The priority model resolves selection before any handler runs, which is simpler and enough for everything wanted so far. Fall-through remains available as an additive future step if a handler ever needs to conditionally decline.

## Open questions

- A dedicated high-priority trigger for a global escape hatch (the ctrl-q case), gated on modifier state so the chord is one recognizable event.
- Whether a binding site should be able to override its trigger's priority, for the rare case where the same trigger type wants different precedence in different places.
- Whether accumulate's `DuplicateTrigger` is still the right guard now that overlap is resolved by priority. Two equal triggers at the same priority on one path is still ambiguous and still an error; the question is only whether that is the useful line.
