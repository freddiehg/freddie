# Invalidation: posts react on the way up

Not done. The descent schedules, the ascent executes. Pres read on the way down and reshape nothing, which fixes the set of handlers. Posts run on the way up, leaf to root, each handed a `Validity`, and a post mutates only when valid. The exclusive winner is the post that reshapes; every other post runs after it and learns it was invalidated. This refines `handler-kinds.md` and `exclusive-as-post.md`.

## Order

Posts run AFTER the winner reshapes. The winner can invalidate a nested handler, so a post has to run after the reshape to see it. Running before hides the reshape and brings back the wasted arm.

## Validity

`Valid` carries the node; `Invalidated` carries nothing. Node access lives in `Valid`, so touching a stale node does not compile.

```rust
struct Valid<'n, N> {
    node: &'n mut N,
    handled: bool,   // a handler matched at or below me
}
enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated,     // my node was replaced
}
```

`handled` records that a handler MATCHED below, not that state changed. Proving a change needs tracking `get_mut`, which is possible and deferred, so `handled` stays conservative.

`only_if_valid` unwraps the valid side. A post that reads `handled` or acts on `Invalidated` matches directly.

```rust
fn only_if_valid<N>(f: impl FnOnce(&mut N) -> Vec<Effect>) -> impl FnOnce(Validity<N>) -> Vec<Effect> {
    move |v| match v {
        Validity::Valid(valid) => f(valid.node),
        Validity::Invalidated => Vec::new(),
    }
}
```

It is a from-below signal. You see reshapes below you, whose handlers ran first; a reshape above you runs later and cannot reach effects you already returned.

## pre is carriage, not detection

A `pre` carries a value read on the valid descent into a post effect, for when the ascent reshapes the source. Detection is `Validity`'s job. No mercury post needs carriage, so `pre` stays unbuilt.

## The rearm is a lone post

Mint in the post, gated by validity. No `arm`, no threaded value, no pending timer.

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;   // replaces the old guard, cancelling the old timer
    vec![schedule]
}
```

`Valid` (activity, or nav within the set): rearm. `Invalidated` (nav out of the set): `only_if_valid` skips, and the dropped node's guard cancels the old timer. No wasted arm.

## The overlay proves the minimal model

`OverlayLayer` wraps the layer-enum and hides the overlay when a handler ran below it. One post on the outside, no hide line in every navigation handler.

```rust
#[post(AnyKey => hide_on_change)]
fn hide_on_change(v: Validity<OverlayLayer>) -> Vec<MercuryEffect> {
    match v {
        Validity::Valid(valid) if valid.handled => vec![hide_overlay()],
        _ => vec![],
    }
}
```

It emits an effect and mutates no ancestor, so it needs `post` + `Validity` and nothing else: no owned path, no scheduler. This is the case that justifies the model, and only the minimal model.

## Not justified: the scheduler

Every handler here returns effects or mutates its own node. None mutates an ancestor. The re-derivable-path scheduler is needed only when several handlers each mutate an ancestor in one dispatch, the `A { overlay, layer }` shape: an additive writer of `A.overlay` beside an exclusive writer of `A.layer`. Under single-owner that cannot happen, because reaching the root mutably consumes the path, the path consumes once, and so owned equals exclusive. No mercury case needs it. Build it after a non-winner is proven to mutate root state that cannot relocate to its own node.

## Effects survive invalidation

A post that runs before a shallower reshape keeps its effects. In `root(a) -> layer -> NavLayer(b)`, `b` returns `fx`, then `a` swaps the layer; `fx` stays, because it is an owned `Vec<Effect>` `a` cannot reach into.

## Refines the other docs

- `handler-kinds.md`: `Nested` (Handled/Missed) becomes `Validity` on the post; `pre` becomes read-only, or absent.
- `exclusive-as-post.md`: the winner is the post that reshapes; every other post reads `Validity`.

## Open

- `handled` is a bool. Precise change detection (track `get_mut`) and level-granular invalidation ("reshaped up to depth N") are deferred.
- The framework always calls a post, handing `Invalidated` to a reshaped one, so the post decides.
- Multiple children: an enum (one active) works; a product (several live) needs a join in `into_parent`.
