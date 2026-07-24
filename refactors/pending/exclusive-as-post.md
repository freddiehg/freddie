# Exclusive handlers are a post that reads `Nested`

Not done. Thesis: the exclusive `#[bind]` is not a third handler position beside `pre`/`post`. It is a `#[pre_post]` whose post checks `Nested` — "did something deeper already handle this?" — and acts only when nothing did. Deepest-wins, the entire point of exclusivity, falls out of that check plus a `Nested` the acting post raises from `Missed` to `Handled`. This builds on `handler-kinds.md`.

## The insight

A post already receives `Nested`. Thread it MUTABLY up the ascent, `Missed` at the leaf:

- an exclusive post whose `pre` matched and whose `nested` is still `Missed` is the deepest matcher: it acts, and raises `nested = Handled`.
- every shallower exclusive post then reads `Handled` and does nothing.

Deepest-wins is exactly that: the deepest matcher reaches `Missed` first — nothing below it won — takes it, and shuts the door on the ancestors. No search on the unwind, no `Break`, no short-circuit. One sweep, a monotone flag rising through it.

## Desugaring `#[bind]`

```rust
#[bind(Key::KeyN.down() => to_nav)]
```

is

```rust
#[pre_post(Key::KeyN.down() => (
    |_ev, _node| ((), vec![]),   // pre: the trigger check. Binds `opt = Some(())` iff the key matched.
    exclusive(to_nav),           // post: run `to_nav` iff we matched AND nobody deeper won.
))]
```

The `pre` carries no value — the winner needs only the fact that it matched, which `opt: Option<()>` records. The `post` is the handler gated on `Missed`:

```rust
// `exclusive(h)` expands to this post:
|(), node, nested: &mut Nested| {
    if *nested == Nested::Missed {
        *nested = Nested::Handled;   // shut the door on the ancestors
        h(node)                      // the handler body: reach the root, mutate, return effects
    } else {
        vec![]                       // a deeper exclusive already won; defer
    }
}
```

`opt = Some(())` is what makes the post run at all (its `pre` matched); `nested == Missed` is what makes it WIN. Two gates, both already in the machine.

## What this deletes

- `ControlFlow`, `Break`, `Continue`. The ascent no longer reports fired-versus-missed; it runs every level's `on_into_parent` and carries `Nested` and the batch. `dispatch` returns `Vec<M::Effect>` and nothing else.
- The separate exclusive search on the unwind. There is no "try this node's binds after the subtree missed" — a node's exclusive binds ARE posts, run like every other post, distinguished only by the `Nested` gate.
- `#[bind]` as a code path in the macro: it lowers to `#[pre_post]` and shares its generated body.

## The one wrinkle: the winner acts at the root, not its level

A plain post acts at its own node. An exclusive winner's body (`to_nav` → `set_layer`) mutates the ROOT, and `set_layer` replaces the very layer every post node lives in — so it cannot run mid-sweep without invalidating the nodes the posts above it still need. It has to land after every post, at the root.

That is already where `complete` puts it today: `complete` climbs running the posts, THEN `set_layer`s at the root. So the exclusive post is a post whose gated body targets the root and lands last. Concretely, `h(node)` is not run in place; the winning post hands its root-action up, and it is applied once the sweep reaches the root — the same ordering `handler-kinds.md` already gives (`A_pre, B_pre, winner, B_post, A_post`), now expressed as one sweep instead of a `complete` driven by the winner.

So the honest reduction: an exclusive handler is a `(pre, post)` where the `pre` is the trigger check, the `post` is `Nested`-gated, and the post's body is a root-action rather than a node-action. The exclusivity is free (it is the `Nested` read the post already gets); the root-targeting is the `ascend`/`complete` the handler already does. Nothing new, one fewer position.

## Open

- `Nested` threaded as `&mut` through the sweep, vs a post that RETURNS whether it won and the framework raises the flag. The `&mut` reads simplest; the return keeps the batch the only mutable thread.
- How the winning root-action reaches the root and lands last: carried up as a value (a `FnOnce(&mut Root) -> Vec<Effect>`, applied at the root) vs the winning post driving `complete` in place as today. The first unifies the shape but carries a mutation closure; the second keeps `complete` but leaves the winner post special.
- Whether `pre` even runs for a `#[bind]`-lowered node, or the trigger check collapses into the post's own `is_matching` on the descent-bound `opt`.
- Naming: if `#[bind]` is sugar, does it stay spelled `#[bind]`, or become `#[pre_post]` with an `exclusive` post helper.
