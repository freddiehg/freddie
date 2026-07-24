# The handler model: pre, exclusive, post

Not done. This is the spec for what a dispatched handler can be. It replaces the also-bind design (`refactors/past/also-binds.md`): an also-bind is just this model's `pre` with a no-op `post`, so it is not a separate feature. The return-home wrapper (`timed-layer-wrapper.md`) is the first user.

## The three positions

A node can hang three things off an event:

- exclusive (`bind`): at most one fires per event, the leafward-most match; it is the winner and produces effects. Today's only kind.
- pre: runs on the way DOWN, at every node the event passes through, before descending into the child. Node-local.
- post: runs on the way UP, paired with a pre. It consumes what the pre produced.

Pre and post are one paired handler, always together; either half may be a no-op. A bare pre (post is a no-op) is what the also-bind was. A bare post (pre is a no-op) is a plain after-hook. The full pair threads a value from pre to post.

## The guarantee

If a node's `pre` ran, its `post` runs, exactly once. This is what makes the pair safe to reason about: `pre` can take an action it expects `post` to complete or discard, with no path where `pre` fires and `post` silently does not.

## The three cases, and the one that matters

For a node whose `pre` ran, the event's exclusive handler determines how `post` runs:

1. No exclusive handler fires in the subtree — the child missed. The path is intact; the framework unwinds through the node and runs `post`. STAYED.
2. An exclusive handler fires but is node-local — it took `Node<&mut P>`, did not ascend, did not consume the path. The path is still intact, so the framework still unwinds through the node and runs `post`. STAYED.
3. An exclusive handler ASCENDED past the node — it took `Node<P>`, consumed the path north to `&mut Mercury`, and `set_layer`d. The path is gone and the node is dropped by the layer swap. Its `post` cannot run on the unwind. LEFT.

Cases 1 and 2 are the easy ones: the path survives, so the framework runs `post` normally. Case 3 is the only hard one, and the design makes it work rather than ruling it out.

## How case 3 keeps the guarantee: the drop is the post

In case 3 the framework cannot run `post` — but `post`'s job on a leave is to DISCARD what `pre` held, and the drop does exactly that. `pre` holds its intermediate value IN the node; when the ascending handler's `set_layer` replaces the layer, the node is dropped and the held value goes with it. So the guarantee holds as an exclusive-or:

- Stayed (cases 1, 2): the framework runs `post`, which consumes the held value.
- Left (case 3): the node is dropped, which discards the held value.

Exactly one happens for every event whose `pre` ran. The distinction — stayed vs left — is carried by whether an exclusive handler ascended past the node, NOT by reading the after-state. That is why this needs no root before/after comparison and works mid-tree.

## Why this respects the effects invariant

Effects leave for the world only on the STAY path, through the framework's ordered output. The LEAVE path only drops a held value; `Drop` emits nothing. This is the inverse of `drop-emits-effects.md` — drop-DISCARDS-effects — and is trivially ordered and safe. No handler issues an effect that mutates state, and no drop emits an effect; state changes only in `handle`, by a handler, directly.

## The handler API

Both handlers take the node with `parent` borrowed (`Node<&mut P, D>`), so they can `get_mut` their own node and `ascend` to READ the root, but cannot `ascend_mut` to consume it — the borrow is the restriction. The value `pre` produces is NOT threaded through the framework; `pre` stashes it in a field on its own node, and `post` takes it back out. That is what makes the leave case a plain drop: the stash lives in the node, so `set_layer` dropping the node discards it.

- A `pre` handler: `fn(&Event, Node<&mut P, D>)`. It mutates its own node — mints, stashes — and returns nothing; whatever it needs `post` to see, it leaves in a node field.
- A `post` handler: `fn(&Event, Node<&mut P, D>, Descent) -> Vec<Effect>`. It reads what `pre` stashed and emits. `Descent` is the one outcome exposed, an enum rather than a bool so the meaning is named and the set can grow without churning every signature:

  ```rust
  /// What the descent below a post did, for the post to branch on.
  enum Descent {
      /// An exclusive bind fired at or below this node.
      Handled,
      /// Nothing exclusive fired at or below this node.
      Unhandled,
  }
  ```

  It is a semantic fact, not the mechanism — the API does NOT expose whether `post` is being called mid-ascent or on a normal unwind, and there is no `DescendedPast` variant, because that case never reaches a `post` (it is the drop). `Descent` is exposed because it is a clean thing a `post` might branch on; the rearm ignores it.
- An exclusive handler is either node-local (`Node<&mut P>`, does not ascend — the framework keeps the path and can run ancestor posts) or ascending (`Node<P>`, consumes the path to reach the root). Which one separates case 2 from case 3, and it is declared, because the framework must know whether the path survives: the ascending kind is the one that reaches the root to `set_layer`.

## The dispatch-contract change

`bind`'s `Dispatch` gains the pre/post phases and the "run posts on the unwind" behavior. The shape:

- Descend visits every node on the active path; each runs its `pre` (which stashes in the node) before recursing into the child.
- On the way up, each node runs its `post`, EVEN past a node-local exclusive `Break` — a node-local exclusive handler hands the path back, so the unwind continues. `post` receives a `Descent` — `Handled` if an exclusive matched at or below this node, `Unhandled` if not.
- An ascending exclusive handler consumes the path and short-circuits the unwind; the nodes below the ascent are dropped, and their stashed values with them — the leave case, handled by the drop, never a `post` call.

The one thing the framework must thread that it does not today is the path back from a node-local exclusive `Break` so the unwind can proceed; the pre/post values ride in node fields, not the framework. The exclusive winner's effects and every `post`'s effects accumulate into one output.

The exact generated bodies (`dispatch_impl`, `descend_impl`, the derived-level impls) follow from this contract and are written out during implementation; the decisions above — the three cases, the guarantee, the borrowed-node handler shape, the stash-in-the-node value passing, `Descent` as the only exposed outcome, node-local vs ascending exclusive handlers, drop-discards-the-stash — are what the implementation must not re-decide.

## The rearm is the first user

`AndReturnHome` gets a pre/post pair instead of an also-bind:

- `pre` arms a fresh timer — `arm_return_home()` gives a `(guard, schedule_effect)` — and holds BOTH in the node without emitting the schedule. The held value is the schedule effect.
- `post` (stay) emits the held schedule. The timer is reset; the reschedule reaches the world.
- On a leave (case 3), the ascending handler's `set_layer` drops `AndReturnHome`, dropping the held schedule and the guard. Nothing is emitted, nothing is scheduled, nothing self-cancels.

This is the wasteless rearm, on the wrapper, owning its own timer — no root `rearm_after`, and none of the also-bind's self-cancelling arm on the keys that leave. `AndReturnHome` also keeps its exclusive firing (`guard.trigger() => to_home`), unchanged.

## Tests

In `crates/bind/tests/`, a fixture with a pre/post node over the existing tree, its `pre` incrementing a counter and holding a value, its `post` recording what it consumed.

- Case 1: an event that misses the subtree runs `pre` then `post`; `post` sees the held value.
- Case 2: a node-local exclusive handler fires below the pre/post node; `pre` and `post` still both run, and the exclusive winner's effect and the `post`'s effect both appear in the output.
- Case 3: an ascending exclusive handler (one that consumes the path) fires below; `pre` runs, `post` does NOT run, and the held value is dropped — asserted by a drop counter on the held type.
- The guarantee: across all three, `pre`-count equals `post`-count-plus-drop-count.
- The exclusive winner is unchanged: the leafward-most exclusive still wins and still consumes; pre/post are additive around it.

## Status

The design is decided; the bind-core implementation is the work — the pre/post phases, the held-value threading, the path-return on a node-local exclusive `Break`, and the node-local-vs-ascending exclusive distinction. It subsumes the also-bind entirely (`pre` with a no-op `post`), so there is one mechanism, not two, and the rearm is its first user.
