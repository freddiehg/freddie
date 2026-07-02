# passing keys through unchanged

A remapper does nothing to most keys: a few are remapped or consumed, and the rest should reach the app as if the remapper were not there. How to model "these keys pass through unchanged" is the question. There are two shapes, and they interact with dispatch and with whether the keyboard is actually hijacked.

## Two meanings, by mode

Passthrough means different things depending on whether the tap swallows:

- Listen (v1): nothing is swallowed, so a key reaches the app on its own. Passthrough is a no-op; modeling it only marks the key as handled.
- Hijack: everything is swallowed, so passthrough means re-emit the key identically (synthesize it). Passthrough is a real effect.

So the model has to produce something the effect handler turns into a re-emit under hijack and drops under listen. The simplest is one effect, `Passthru(key)` (mercury currently reuses `Type(key)`), that the effect loop re-emits or ignores per mode.

## Shape 1: an explicit wildcard bind

What mercury does today: a layer binds `AnyKey => passthru`, a wildcard trigger that matches any key, and `passthru` emits the key.

- It reads well: "in typing, any key passes through."
- But dispatch is child-first, so a leaf catch-all shadows ancestor binds. A global `escape` bound on the parent `Layer` never fires under a leaf that matches everything, so mercury excludes `escape` from `AnyKey`. That is a wart: the wildcard has to know about the globals above it.
- Per-key exceptions ("these letters remap, the rest pass through") work only if the specific binds are tried before the wildcard, so ordering within a node becomes load-bearing.

## Shape 2: passthrough as the default

The alternative: do not bind passthrough at all. Treat an unhandled event (dispatch returns `None`) as passthrough, and have the runner re-emit it. A layer then binds only the keys it changes or consumes; everything else falls through and is passed.

- No wildcard and no exclusions. A leaf that binds nothing lets every key bubble, so an ancestor's `escape` still fires, and a key unbound past that is re-emitted by the runner.
- "Consume this key" (swallow, no passthrough) becomes an explicit bind to a handler that returns no effect. The three outcomes are then clear: unbound is passthrough, bound-to-nothing is swallowed, bound-to-something is remapped.
- Per-key exceptions are just binds; no ordering trick.
- Cost: passthrough is implicit. You cannot see "this layer passes the rest through" in the binds, because it is the absence of binds. And the runner has to know the original key to re-emit, so the event has to carry it.

## Dispatch interaction

Both shapes live under child-first dispatch, where a child's bind beats an ancestor's. Shape 1 fights it: a catch-all leaf shadows ancestors, hence the exclusions. Shape 2 works with it: leaves bind little, and ancestors plus the default catch the rest. That is the main reason to prefer shape 2 for a remapper, where passthrough is the common case and remaps are the exception.

## How it could work (post-v1)

Two levers, and the second is the real win:

- Swallow only what the state binds; pass the rest. The tap callback returns the original event (pass) or drops it (swallow). If it swallows only the keys the current state binds and passes the rest, an unbound key continues natively: no swallow, no re-emit, no dispatch, just the callback firing and a set lookup. The callback needs the active trigger set to decide, which is what `accumulate` produces; share it with the tap thread and update it on state change. This is the perf win: passthrough is the common case, and now it costs a lookup instead of a channel round-trip plus a synthesized re-emit.
- Not firing the callback at all. A `CGEventTap` fires for every key of its type; you cannot ask it to skip specific keycodes. The only way the OS passes keys without handing them to you is to register just the ones you want (`RegisterEventHotKey`), so the rest never reach the process. But that is for fixed hotkey combos, not layer-based remapping, and you would re-register on every layer change. Not worth it for a modal remapper.

So the model shifts from "swallow everything and re-emit passthrough" to "swallow only what the state binds and pass the rest." That makes the swallow decision synchronous in the tap (it reads the active set), while dispatch of the swallowed keys stays async.

Explicit passthrough still has a role on top of this. A bind whose target is `Passthru` rather than a handler (`Key("x") => Passthru`) is how a child overrides an ancestor that remaps `x`: that key is in the active set, so it is swallowed and dispatched, and dispatch re-emits it. Implicit passthrough (a key nobody binds) is the unbound default, passed natively by the tap. So `bind` binds an enum of handler-or-passthrough.

## A middle case: pass a named set through

"These specific keys pass through, and nothing else here is bound" is either a small group trigger (`OneOf(["a", "s", "d"]) => passthru`, shape 1 without the catch-all problem, since it does not match the globals) or, under shape 2, just those keys left unbound. The group trigger is worth it only when you want the passed-through set to be visible in the binds.

## Recommendation

For a remapper, shape 2 (passthrough is the default and the runner re-emits unbound keys) is the better fit: it matches the common case, avoids the wildcard-versus-global wart, and makes remaps and consumes explicit. Shape 1 stays useful when a layer should visibly declare "everything here passes through" as a binding rather than an absence. mercury uses shape 1 (`AnyKey => passthru`) today; moving to shape 2 drops the `AnyKey` exclusion and adds one step to the runner: on a `None` from dispatch, re-emit the key.
