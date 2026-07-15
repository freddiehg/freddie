# passthrough and modifiers move to the root

Not built, and the deferred follow-up: doing ALL passthrough on the root. `held-modifiers.md` moves the modifier state to the root and keeps the per-layer catch-alls; this doc takes passing-keys-through off the layers entirely and puts it, and the modifier handling, in one root catch-all. The end state is that `TypingLayer` and `Paused` bind only their own commands and are otherwise inert markers; the root does all the passthrough and all the modifier tracking; and modifier keys are not special anywhere. The `HeldModifiers` struct itself is `held-modifiers.md`'s; this doc only moves who updates it.

## What is still on the layers today

Even after `passthrough-count.md`, the layers carry two things they should not:

- Passthrough responsibility. `TypingLayer`/`Paused` hold a `PassthroughLayerGuard` and the tap keeps originals while any guard is live. The layer is still the thing that declares "keys pass through now."
- Modifier awareness. Something has to update `held` as modifiers go down and up, and today that is a layer catch-all (typing's `AnyKey`, the paused arm's `AnyKey`) branching on whether the key is a modifier. So modifiers ride through a layer's handler.

Both are the layer doing work that is not layer-specific. Passthrough is a global fact; modifier state is a global fact. They belong to the root.

## Modifiers are not special: `NonModifierKey`

Rename the catch-all trigger from `AnyKey` to `NonModifierKey` and make it match every key EXCEPT a modifier:

```rust
pub struct NonModifierKey;

impl EventTrigger for NonModifierKey {
    type Event = KeyEvent;
    fn is_matching(&self, ev: &KeyEvent) -> bool {
        !ev.key.is_modifier() // meta/ctrl/alt/shift, left and right
    }
}
```

Now no layer's catch-all ever sees a modifier. A modifier key always falls past the active layer to the root, which is the one place that tracks `held` and decides whether to emit it. This is `modifier-keys.md`'s point made real: modifiers are ordinary keys, and precisely because they are ordinary, no layer gets to treat them as commands, so they route to the single owner. (`Key::is_modifier` is a small helper in `freddie_keys`.)

## The root owns passthrough

With synchronous dispatch the root sees every key and knows the active layer, so it does not need the layers to raise a flag for it. The root decides passthrough itself:

- A modifier key: the root records it in `held` always (so `held` stays accurate whatever the layer), but emits it -- passes it through -- ONLY when a passthrough layer is active. In a command layer a modifier is swallowed like every other key. That is the whole point: command mode swallows everything, modifiers included; passthrough is the only place a modifier reaches the app. So a modifier is treated exactly like any other key for emit-vs-swallow (gated on passthrough), and is special only in that it also updates `held`. `held` is one struct on the root; see `held-modifiers.md`.
- A non-modifier key the active layer did not bind: if the active layer is a passthrough layer (typing, or paused), the root passes it through; otherwise (a command layer) it is swallowed. This replaces the per-layer emit-everything catch-alls.
- A non-modifier key the active layer bound: the layer's command runs, as now.

So "is this a passthrough layer" is asked of the active layer's identity at the root, not answered by a guard the layer holds. `ActivePassthroughLayer` and `PassthroughLayerGuard` from `passthrough-count.md` collapse into a predicate on the state tree (`is the active layer typing, or are we paused`), and the count exists only to the extent that paused-over-typing is two truths at once, which the root can read directly from the tree rather than from a counter.

## `AnyKey => passthrough` on the root (not yet)

The concrete form of "the root owns passthrough" is a catch-all bound at the root: `AnyKey => passthrough`. Today the paused arm passes through explicitly with its own `AnyKey => pass_through`; this hoists that one binding to the root, and then typing and paused stop binding a catch-all at all.

How it composes with the layers: dispatch is leafward, so a key the active layer binds runs the layer's command, and a key the active layer does NOT bind falls past it to the root's `AnyKey`. That is the fall-through. The root's handler is then the single place that passes a key through.

The catch it has to handle: the root's `AnyKey` is reached in EVERY layer, including command layers, but command layers should still swallow their unbound keys, not pass them. So the handler is not "always emit" -- it passes through only when the active state is a passthrough one (typing, or paused), and swallows otherwise. That check is the same passthrough predicate as above, now consulted inside the root handler rather than by the tap. (And modifiers, via `NonModifierKey`, reach this same root catch-all regardless of layer, which is how the root gets to record `held` and emit them.)

Deferred: worth doing, but "not yet" -- it depends on the fall-through-versus-multi-cast decision below, and on the passthrough predicate being readable at the root.

## The layers become markers

`TypingLayer` and `Paused` lose their catch-alls and their guards. They keep only their real commands:

- `TypingLayer`: `escape` / `cmd`-`escape` to leave. Nothing else; the root passes the rest through.
- `Paused`: the `cmd`-`alt`-`p` unpause. Nothing else; the root passes the rest through.

They exist to say "we are in a passthrough state" by their type, and to hold their commands. All the passing-through is the root's.

## The tension to resolve: one handler per event

This is why it is a follow-up and not a line in the first doc. Dispatch runs ONE handler per event (leafward). For the root to both track a modifier AND let a layer act on the same key, or to run its passthrough fall-through after the active layer declined the key, one event has to reach more than one handler. That is the `no-clobber.md` decision.

Two shapes, and the doc has to pick:

- Fall-through: keep one-handler dispatch, and make "no layer bound this key" hand the event up to the root, which then runs its passthrough/modifier handler. The root is the last resort, reached only when nothing leafward claimed the key. `NonModifierKey` already guarantees modifiers reach it (no layer catch-all claims them); the fall-through guarantees unbound non-modifiers do too.
- Multi-cast: let the root's modifier/passthrough handler run in addition to the layer's, for every event. This is the actual `no-clobber` change, and it makes "modifier state at the root" trivial (the root sees every key unconditionally), at the cost of the dispatch model no longer being one-handler-per-event.

The fall-through is smaller and keeps the dispatch model; the multi-cast is cleaner conceptually and is the thing global modifier state really wants. Deciding between them is the crux of this doc.

## Cleaner with multiple children: an `IfActivePassthru`

"Is a passthrough layer active" is checked imperatively above -- the root handler asks the predicate before passing a key through. That is a branch standing in for structure. The cleaner shape makes the predicate be whether a child exists: a node present iff a passthrough layer is active, holding only the passthrough catch-all.

```rust
// a child of the root, present only while a passthrough layer is active
#[bind(AnyKey => passthrough)]
pub struct IfActivePassthru;
```

The root would then resolve into TWO children at once: the active layer (its commands) and this `IfActivePassthru`. A bound key hits the layer; anything else falls through to `IfActivePassthru`'s catch-all; and when no passthrough layer is active the child is simply absent, so there is no catch-all and the key is swallowed. The predicate moves out of a handler and into whether the child resolves.

laserbeam does not support this. It resolves into exactly ONE active child (an enum variant, or a single `#[resolve_into]`). The conditional-presence half is the `Option<Child>` / fallible resolve from `laserbeam-state-controlled-children.md`, but here the root keeps its layer child too, so it needs MULTIPLE simultaneous children, which is that doc's non-goal and is the `no-clobber.md` multi-cast decision by another name (more than one child seeing the event). So `IfActivePassthru` is the target shape once laserbeam has multiple children; until then the root handler branches on the predicate.

## Open questions

- Fall-through versus multi-cast (the `no-clobber.md` decision), and whether the root's passthrough/modifier logic is a real last-resort handler or a per-event side-channel. `IfActivePassthru` is the structural form of the multi-cast answer.
- Whether the `ActivePassthroughLayer` count survives at all, or is replaced by reading the active layer (and paused-ness) off the tree directly, or by `IfActivePassthru`'s presence.
- The stuck-modifier hazard when a `cmd` is passed through but its up is later swallowed. The single `held` on the root is a precondition, not the fix; the corrective-emit wiring is the work (`held-modifiers.md`). (The keep-vs-drop for command keys is settled in `passthrough-count.md`: `is_active()`-after-dispatch, not the tap needing the dispatch result.)
- The single `held` struct, its update site, the emitter-flags reconciliation, and the `u128` form all live in `held-modifiers.md`.
