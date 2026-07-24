# Also-binds

Not done. This is the prerequisite for `timed-layer-wrapper.md`: the return-home wrapper resets its idle timer with an also-bind, so also-bind dispatch has to exist first. This records the mechanism, its syntax, and its one accepted cost.

## What it is

Today a node's own binds are exclusive. Dispatch descends into the active child first; if the child handles the event it `Break`s and the node's own binds never run; only on a child miss does the node try its own binds, and the first match `Break`s. At most one handler runs per event, the leafward-most.

An also-bind is a second kind of bind that fires IN ADDITION to whatever else matched, not instead of it. A node with one contributes its effects on every event that reaches it, whether or not a child (or the node's own exclusive binds) also handled that event.

The motivating case is the return-home wrapper: `AndReturnHome` resets its idle timer on every key that reaches it — a node-local effect on every event through its subtree. See `timed-layer-wrapper.md`.

## Syntax

Two attributes on a node, told apart by name. A node may carry both; `also_bind` fires first (pre-descend), `bind` after (post-descend on a child miss):

```rust
#[bind(Key::Escape.down() => to_home)]        // exclusive: fires iff nothing deeper did
#[also_bind(AnyKey => rearm)]               // also-bind: fires alongside whatever did
```

Order among the attributes never matters: two exclusive binds cannot share a trigger on the active path (the check forbids it), and the two kinds run in fixed phases (below), so declaration order changes nothing.

## The dispatch-contract change

This is the load the feature carries. Exclusive dispatch short-circuits:

```rust
// today
fn dispatch(path, event) -> ControlFlow<Output, Path>;
// Break(out) = handled, stop the walk. Continue(path) = missed, parent's turn.
```

Also-binds must run at every node on the active path, but `Break` stops the walk, so an exclusive winner deep in the tree would prevent every ancestor's also-bind from running. To have both, dispatch stops short-circuiting: it walks the whole active path, like `accumulate` already does for the check, threading an accumulator. Also-binds push into it at each node; the single exclusive winner is recorded (and two exclusive matches on one path is the clobber error, as today).

```rust
// with "or"
fn dispatch(path, event, out: &mut Output) -> ControlFlow<(), Path>;
// also-binds push to `out` as the walk passes each node;
// `Break(())` records that THIS node's exclusive bind won (walk continues for also-bind);
// the top-level driver returns `out` (None only if nothing, exclusive or also-bind, matched).
```

So the core change is: dispatch goes from short-circuit to full-path walk with an accumulator, and `Output` is built up rather than returned by the one winner. Paths are short, so the cost is negligible at runtime; the cost is the rework of `bind`'s central contract and every generated body.

## The proc-macro changes

`dispatch_impl` in `bind_macro` partitions a node's binds into exclusive (`#[bind]`) and also-bind (`#[also_bind]`), and emits three phases instead of one. Today it emits `recurse-into-child; own exclusive checks; Continue`. With "or" it emits:

```rust
fn dispatch<'a>(mut path: Path, event: &Event, out: &mut Output) -> ControlFlow<(), Path> {
    // 1. Also-binds, before descending. Each gets the node with `parent` borrowed
    //    (`&mut path`), so it can mutate this node and read upward but cannot consume the path
    //    that phase 2 needs.
    #( if let Some(ev) = try_from(&event) {
           let trigger = #or_trigger;
           if is_matching(&trigger, ev) {
               out.extend(#or_handler(ev, Node { parent: &mut path, data: () }));
           }
       } )*

    // 2. Descend into the active child, which pushes its own also-bind + exclusive into `out`.
    let child = <Child>::dispatch(#child_path, event, out);
    path = #recover_or_break(child);   // Break bubbles the exclusive winner's `Break(())`

    // 3. This node's exclusive binds, tried only if no deeper exclusive bind won.
    #( if let Some(ev) = try_from(&event) {
           let trigger = #excl_trigger;
           if is_matching(&trigger, ev) {
               out.extend(#excl_handler(ev, Node { parent: path, data: () }));
               return ControlFlow::Break(());
           }
       } )*

    ControlFlow::Continue(path)
}
```

`accumulate_impl` (the check) is unchanged in structure — it already walks the full path — but its trigger-collection has to treat the two kinds differently (below). `descend_impl` and the derived-node impls take the same `out: &mut Output` parameter and the same `Break(())`/`Continue` shape.

## Why an also-bind runs first, and what that costs the handler

The ordering is forced, not chosen: running an also-bind after the child would stop it being also-bind at all. If the child handles the event, its handler ascends to the root and consumes the path — there is nothing to hand back up, which is why a handled child returns `Break` with no path. So once the child has handled the event, no ancestor holds a path and can run nothing. An ancestor's also-bind is therefore only guaranteed to run if it runs before the descent. Put it after, and it runs only when the child returned the path, i.e. only when the child missed — and "the node's own bind fires iff the child missed" is precisely exclusive dispatch. An also-bind placed after the child collapses into an ordinary exclusive bind and buys nothing. Pre-descend is what makes "or" mean "or".

An inverted ordering is imaginable and worth noting as a future possibility, though not taken: run the leaf-most exclusive handler first and defer the also-binds to fire on the way back up, with only the exclusive winner ever consuming the path and the also-binds scheduled and applied afterward. It is a heavier machine, and it does not solve the extra work below. An also-bind deferred until after the ascent still cannot know why the ascent happened — whether the leaf changed the layer or did something unrelated — so it still acts blindly and pays the same cost. Ordering is not the lever on that cost.

Running before the child means the also-bind handler must not consume the path — phase 2 still needs it. It keeps the same `Node<parent, data>` shape an exclusive handler has, but `parent` is the path by mutable reference rather than by value: `Node<&mut P, Data>`. That enforces the restriction for free — `into_parent` and `ascend_mut` take `self` by value, so they are uncallable through the `&mut`; `get`, `parent`, `ascend`, and `get_mut` take `&self`/`&mut self`, so they remain. The borrow IS the restriction.

```rust
// exclusive handler: `parent` is the owned path, so it can ascend to the root and `set_layer`
fn to_home<'a, E, P: AscendMut<MercuryPath<'a>>>(ev: &E, node: Node<P, ()>) -> Vec<Effect>;

// also-bind handler: `parent` is `&mut` the path, so it can `get_mut` its own node and `ascend` to
// READ the root, but not `ascend_mut` to mutate it, nor `into_parent` to consume it; `data` stays
fn rearm<'a>(ev: &KeyEvent, node: Node<&mut AndReturnHomePath<'a>, ()>) -> Vec<Effect>;
```

Keeping the `Node`, rather than passing a bare `&mut path`, holds the handler shape uniform and carries the derived `data` an also-bind on a derived level would want; `parent: &mut P` in place of `parent: P` is the whole of the difference.

The capability is exactly `Ascend` (read up, root included) plus `get_mut` (mutate this node), and not `AscendMut` — the split the ascend-by-ref work already draws. So an also-bind handler can read anything up to the root and mutate its own node, but it cannot reach the root to mutate it: `set_layer`, foregrounding, the whole existing handler set is out, because two handlers cannot both hold the path to the root and the also-bind is not the one that gets to. The one seam is the root itself, whose path is `&mut Mercury` rather than a `PathMut`; there a trait (implemented for `&mut PathMut<N, P>` and for `&mut &mut Root`) unifies the surface, the same reason `Ascend`/`AscendMut` are traits.

## What "or" really costs: the impossibility of contradiction

It is easy to read the clobber check as the thing exclusive dispatch buys. It is not. The thing it buys is that exactly one handler is the sole authority for an event: one handler is one set of effects from one author, so there is never a second effect to contradict it. The check is only the structural guard that keeps that true — no two exclusive triggers collide, so no event ever has two claimants.

Also-bind breaks the authority, which is worse than losing a check. Now the also-bind's effects and the subtree's effects both apply to one event, and whether they contradict is a semantic question about what the effects mean against the current state, not a trigger collision. The check sees only triggers, so it structurally cannot see this. Nor can it be made to by enumerating states: it already declines to check state-dependent triggers — a closure trigger is excluded from clobber detection precisely because its value is read from state at dispatch, so there is no static claim to compare — and effect-contradiction is a level past that.

So an also-bind handler appears terminal but is not. In the trigger set it looks like a terminal — one trigger, one claim — so the check either flags its intended overlap as a clobber (a false positive) or exempts it and goes blind. Neither is right, because the also-bind is not competing for the one slot; it is adding to the outcome, and "adding" is a thing the model has no vocabulary for.

The real cost, then, is that contradiction moves from statically impossible to runtime-only and unenumerable. Under exclusive dispatch two effects cannot contradict, because there is only ever one. Under also-bind they can, and whether they do depends on state that cannot be reasoned about ahead of time. An also-bind is sound only when its effect is provably independent of everything else on the path, and independence is not checkable in general — so an also-bind returns you to hand-verifying each one, which is the discipline the check existed to delete.

## The rearm is the first user

`AndReturnHome` carries one bind of each kind:

- exclusive `|p| p.get().guard.trigger() => to_home` — the firing, on a `TimerFired` event, reaching the root through `go_home`. Post-descend, as today.
- also-bind `AnyKey => rearm` — resets the timer on every key that reaches the wrapper. Pre-descend, node-local: it mutates `self.guard` and emits the reschedule, nothing more.

The also-bind is a sound one under the rule above, because its only interaction with the rest of the path is benign. On a key that stays, it resets the clock, which is the whole point. On a key that leaves or transitions, it fires anyway — it cannot know why the ascent is happening — and arms a fresh timer that the leaving handler then drops, so the schedule self-cancels. That is the extra work: a wasted arm on the keys that leave. It is a discarded effect, not a contradiction, and it is the price of putting the reset where the timer lives rather than reconstructing the before/after check at the root.

So `handle` loses the rearm entirely: no `rearm_after`, no `activity_token`, no `Layer::rearm_timeout`. `handle` is just `dispatch`, and the also-bind does the resetting. See `timed-layer-wrapper.md`.

## Status

Scheduled — the prerequisite for the return-home wrapper. The work: rework `bind`'s `Dispatch` contract from short-circuit to full-path accumulator; add the `#[also_bind]` attribute, its pre-descend phase, and its `Node<&mut P, Data>` handler shape; and teach the check to hold also-bind triggers exempt from the collision rule. The cost is that rework plus the impossibility of contradiction: an also-bind is sound only when its effect is provably independent, which is not checkable, so each one is hand-verified. The wrapper's rearm is the first, and its independence is the benign self-cancel above.
