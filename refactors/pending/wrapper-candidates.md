# What else is a wrapper

Discussion, not scheduled. `AndReturnHome` (`timed-layer-wrapper.md`) models the return-home timer as a wrapper: a node that owns some state and binds, wraps an inner subtree, and (via an also-bind) acts on every event through it. The question here is what else that shape fits, and whether wrappers compose. The short version: they compose cleanly, but the overlay is not actually a wrapper, and the reason it isn't is the useful part.

## Wrappers stack, and their also-binds commute

Nesting wrappers is fine. `root ▸ Overlay ▸ Layer ▸ AndReturnHome ▸ ...` is a legal tree; each wrapper owns its own binds, and no-clobber holds as long as their triggers are disjoint. On one event, every wrapper's also-bind on the active path fires — they run pre-descend, root-to-leaf, and none can prevent another (`also-binds.md`, "Also-binds compose"). So `AndReturnHome`'s `stay` and an overlay wrapper's `close` both fire on one key, commutatively, because each mutates only its own node and emits independent effects.

The worry that an also-bind might stop `toggle_overlay` is unfounded: `toggle_overlay` is exclusive, and an also-bind never `Break`s, so it cannot short-circuit it. The one real composition constraint is between two EXCLUSIVE binds — they cannot claim the same trigger on the path (clobber) — so an outer wrapper owning `o` exclusively means the inner layers must not bind `o`.

## The overlay is not a wrapper

The overlay looks like a candidate — it owns state (`Mercury.overlay: Option<TimerGuard>`), it binds `o => toggle_overlay` (today in all five layers) and the dwell firing, and it hides on every layer change. One could hoist all that into an outermost `AndCloseOverlay` around `Layer`, deduping the five `o` bindings into one. And it would compose with `AndReturnHome` — pressing `o` in nav would fire `stay` (reset the timer) on the way down and the wrapper's exclusive `o` on the way up, both correct.

But its state lifecycle does not match a subtree, and that is what a wrapper is for. `AndReturnHome`'s timer is born when you ENTER a timed layer and dropped when you LEAVE — its life is subtree membership, which is exactly why the wrapper owns it. The overlay is one object across ALL layers, and it is hidden by layer CHANGES, which `set_layer` manages at the root. Hoisting the state into a node does not remove that coupling; it relocates it: `set_layer` would have to reach into the `AndCloseOverlay` node to hide on every transition, so the root still drives the overlay's lifecycle, now through a node instead of a field.

So the overlay splits into two things, and only one of them is wrapper-shaped:

- The `o` binding is duplicated five times and wants deduping — but that is a binding lift (bind `o` once, above the layers), not a state wrapper. It needs no node to own state.
- The overlay STATE is global and root-managed, so it stays `Mercury.overlay`. Making it a node buys nothing and tangles `set_layer`.

The test this yields: a wrapper is right when the concern's STATE lives and dies with a subtree — constructed on entry, dropped on leave. When the state is global and its lifecycle is driven from the root (a layer change, a mirrored external fact), it is root state, and at most its BINDINGS lift.

## Modifier tracking is a wrapper-free also-bind

`maybe_pass_through` (root, exclusive `AnyKey`) does two jobs today: it records held modifiers on every key, and it passes the key through in a passthrough layer. The first is a textbook also-bind — `AnyKey => track_held` at the root, firing on every key, mutating only root state, independent of everything else. Splitting it out would let `maybe_pass_through` do one thing (passthrough) and the also-bind do the other (tracking), which is cleaner than the current double duty.

Note there is no wrapper here at all: the concern is global (every key, every layer), so it is a root also-bind, not a nested node. This is the other end of the spectrum from `AndReturnHome`: same also-bind mechanism, no subtree, no wrapper.

## Not wrappers

- The `jk` run (`typing_state.jk`) is typing-specific state and logic. It applies only in the passthrough layer, but it is a state machine the layer reads, not a cross-cutting effect on every event, so it stays where it is.
- The front app and window frames are external truth mirrored into root state under the idempotence rule (CLAUDE.md). Their lifecycle is "the OS changed it," seeded and event-fed, not subtree membership. Root state, not wrappers.

## The spectrum

Three shapes, one mechanism:

- Subtree-scoped state + an also-bind on every event through it: a wrapper. `AndReturnHome`.
- Global state + an also-bind on every event: a root also-bind, no wrapper. Modifier tracking.
- Global state, root-managed lifecycle, duplicated bindings: root state plus a binding lift, no also-bind and no wrapper. The overlay.

"Is it a wrapper?" reduces to "does the state live and die with a subtree?" Only the first shape does.
