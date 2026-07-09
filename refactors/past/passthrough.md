# passing keys through unchanged

A remapper leaves most keys alone: a few are remapped or consumed, and the rest should reach the app as if it were not there. Two things to settle: how the runner performs passthrough, and how the model expresses it. The first has a hard constraint that mostly decides the second.

## Passthrough must stay on the ordered path

The tempting shortcut is to pass an untouched key straight through the tap (native, synchronous) and only swallow the keys you remap. It reorders. Swallowing is async: a swallowed key takes the channel round-trip through dispatch and comes back out as a re-emit, while a natively-passed key reaches the app immediately. So if `a` passes natively and `b` is remapped to `B`, typing `a b a` can land as `a a B`: both `a`s go straight through while `B` is still in flight. The moment any key on a layer is remapped, passing the other keys natively is unsound.

There is one correct model: swallow everything, and put every key on the single ordered effect pipeline. Passthrough is then a real effect (re-emit the key identically), not a native pass, and it stays in order with the remaps because it is on the same path. This is why the "swallow only what's bound" optimization and its `ArcSwap` are out (keyboard-capture.md): they exist to pass unbound keys natively, which is exactly what reorders.

The cost is a channel round-trip plus a re-emit per passed key. For a single keyboard that is microseconds, well under perception (event-loop.md), so ordering correctness beats the native-pass shortcut.

## What still varies: how the model expresses it

Performance is now the same either way (every key is swallowed and re-emitted in order). The only open question is whether the model declares passthrough or leaves it implicit.

## Shape 1: an explicit wildcard bind

What mercury does today: a layer binds `AnyKey => passthru`, a wildcard trigger that matches any key, and `passthru` re-emits it.

- It reads well: "in typing, any key passes through."
- But dispatch is child-first, so a leaf catch-all shadows ancestor binds. A global `escape` bound on the parent `Layer` never fires under a leaf that matches everything, so mercury excludes `escape` from `AnyKey`. That is a wart: the wildcard has to know about the globals above it.
- Per-key exceptions ("these letters remap, the rest pass through") work only if the specific binds are tried before the wildcard, so ordering within a node becomes load-bearing.

## Shape 2: passthrough as the default

The alternative: do not bind passthrough at all. Treat an unhandled event (dispatch returns `None`) as passthrough, and have the runner re-emit it. A layer then binds only the keys it changes or consumes; everything else falls through and is re-emitted.

- No wildcard and no exclusions. A leaf that binds nothing lets every key bubble, so an ancestor's `escape` still fires, and a key unbound past that is re-emitted by the runner.
- "Consume this key" (swallow, no re-emit) becomes an explicit bind to a handler that returns no effect. The three outcomes are then clear: unbound is passthrough (re-emit), bound-to-nothing is swallowed silent, bound-to-something is remapped.
- Per-key exceptions are just binds; no ordering trick.
- Cost: passthrough is implicit. You cannot see "this layer passes the rest through" in the binds, because it is the absence of binds. And the runner has to know the original key to re-emit, so the event has to carry it.

Both shapes now perform passthrough identically (swallow, then re-emit in order). The difference is only whether the pass-through set is visible in the binds or is the unbound default.

## Explicit Passthru bind

A bind whose target is `Passthru` rather than a handler (`Key("x") => Passthru`) is how a child overrides an ancestor that remaps `x`: `x` is bound, so it is swallowed and dispatched like anything else, and dispatch re-emits it in order. So `bind` binds an enum of handler-or-passthrough. This is orthogonal to shapes 1 and 2: it is how you say "on this layer, ignore the ancestor's remap of this key and pass it," and it rides the same ordered pipeline.

## A middle case: pass a named set through

"These specific keys pass through, and nothing else here is bound" is either a small group trigger (`OneOf(["a", "s", "d"]) => passthru`, shape 1 without the catch-all problem, since it does not match the globals) or, under shape 2, just those keys left unbound. The group trigger is worth it only when you want the passed-through set to be visible in the binds.

## Dispatch interaction

Both shapes live under child-first dispatch, where a child's bind beats an ancestor's. Shape 1 fights it: a catch-all leaf shadows ancestors, hence the exclusions. Shape 2 works with it: leaves bind little, and ancestors plus the default catch the rest. That is the main reason to prefer shape 2 for a remapper, where passthrough is the common case and remaps are the exception.

## Recommendation

For a remapper, shape 2 (passthrough is the default and the runner re-emits unbound keys) fits best: it matches the common case, avoids the wildcard-versus-global wart, and makes remaps and consumes explicit. Shape 1 stays useful when a layer should visibly declare "everything here passes through" as a binding rather than an absence. mercury uses shape 1 (`AnyKey => passthru`) today; moving to shape 2 drops the `AnyKey` exclusion and adds one step to the runner: on a `None` from dispatch, re-emit the key. Either way, passthrough is a re-emit on the ordered pipeline, never a native pass.
