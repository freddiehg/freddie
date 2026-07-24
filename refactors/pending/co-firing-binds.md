# Co-firing binds ("or" dispatch)

Discussion, not scheduled. `AndReturnHome` does not need this (see `timed-layer-wrapper.md`): its firing is an exclusive bind, and the rearm is a root step. This records how "or" dispatch would be built in the proc macro, and what it costs, so the decision is grounded rather than re-derived.

## What it is

Today a node's own binds are exclusive. Dispatch descends into the active child first; if the child handles the event it `Break`s and the node's own binds never run; only on a child miss does the node try its own binds, and the first match `Break`s. At most one handler runs per event, the leafward-most.

"Or" dispatch adds a second kind of bind that fires IN ADDITION to whatever else matched, not instead of it. A node marked with a co-firing bind contributes its effects on every event that reaches it, whether or not a child (or the node's own exclusive binds) also handled that event.

The motivating shape is a wrapper that wants to act on every event passing through its subtree — the thing `AndReturnHome` looked like it wanted for the rearm before the rearm turned out to belong at the root.

## Syntax

Two attributes on a node, told apart by name, because they mean different dispatch:

```rust
#[bind(Key::Escape.down() => to_home)]     // exclusive: fires iff nothing deeper did
#[bind_or(AnyKey => touch)]                // co-firing: fires alongside whatever did
```

They may coexist on one node. Order among the attributes never matters: two binds cannot share a trigger on the active path (the check forbids it), and the two kinds run in fixed phases (below), so declaration order changes nothing.

## The dispatch-contract change

This is the load the feature carries. Exclusive dispatch short-circuits:

```rust
// today
fn dispatch(path, event) -> ControlFlow<Output, Path>;
// Break(out) = handled, stop the walk. Continue(path) = missed, parent's turn.
```

Co-firing binds must run at every node on the active path, but `Break` stops the walk, so an exclusive winner deep in the tree would prevent every ancestor's co-firing bind from running. To have both, dispatch stops short-circuiting: it walks the whole active path, like `accumulate` already does for the check, threading an accumulator. Co-firing binds push into it at each node; the single exclusive winner is recorded (and two exclusive matches on one path is the clobber error, as today).

```rust
// with "or"
fn dispatch(path, event, out: &mut Output) -> ControlFlow<(), Path>;
// co-firing binds push to `out` as the walk passes each node;
// `Break(())` records that THIS node's exclusive bind won (walk continues for co-fire);
// the top-level driver returns `out` (None only if nothing, exclusive or co-firing, matched).
```

So the core change is: dispatch goes from short-circuit to full-path walk with an accumulator, and `Output` is built up rather than returned by the one winner. Paths are short, so the cost is negligible at runtime; the cost is the rework of `bind`'s central contract and every generated body.

## The proc-macro changes

`dispatch_impl` in `bind_macro` partitions a node's binds into exclusive (`#[bind]`) and co-firing (`#[bind_or]`), and emits three phases instead of one. Today it emits `recurse-into-child; own exclusive checks; Continue`. With "or" it emits:

```rust
fn dispatch<'a>(mut path: Path, event: &Event, out: &mut Output) -> ControlFlow<(), Path> {
    // 1. Co-firing binds, before descending. They run on THIS node only (see the handler
    //    restriction below), so they take `path.get_mut()` and hand it straight back.
    #( if let Some(ev) = try_from(&event) {
           let trigger = #or_trigger;
           if is_matching(&trigger, ev) {
               out.extend(#or_handler(ev, path.get_mut()));
           }
       } )*

    // 2. Descend into the active child, which pushes its own co-fire + exclusive into `out`.
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

## Why co-fire runs first, and what that costs the handler

The ordering is forced, not chosen: running a co-fire after the child would stop it being co-fire at all. If the child handles the event, its handler ascends to the root and consumes the path — there is nothing to hand back up, which is why a handled child returns `Break` with no path. So once the child has handled the event, no ancestor holds a path and can run nothing. An ancestor's co-fire is therefore only guaranteed to run if it runs before the descent. Put it after, and it runs only when the child returned the path, i.e. only when the child missed — and "the node's own bind fires iff the child missed" is precisely exclusive dispatch. A co-fire placed after the child collapses into an ordinary exclusive bind and buys nothing. Pre-descend is what makes "or" mean "or".

Running before the child means the co-fire handler must not consume the path — phase 2 still needs it. The type system enforces this for free: hand the handler the path by mutable borrow rather than by value. `into_parent` and `ascend_mut` take `self` by value, so they are uncallable through a `&mut`; `get`, `parent`, `ascend`, and `get_mut` take `&self`/`&mut self`, so they remain. The borrow IS the restriction — no wrapper type, no annotation.

```rust
// exclusive handler, today's shape: consumes the path, ascends to the root, can `set_layer`
fn to_home<'a, E, P: AscendMut<MercuryPath<'a>>>(ev: &E, node: Node<P, ()>) -> Vec<Effect>;

// co-fire handler: the path by `&mut`, so it can `get_mut` its own node and `ascend` to READ the
// root, but not `ascend_mut` to mutate it and not `into_parent` to consume it
fn touch<'a>(ev: &KeyEvent, path: &mut AndReturnHomePath<'a>) -> Vec<Effect>;
```

The capability is exactly `Ascend` (read up, root included) plus `get_mut` (mutate this node), and not `AscendMut` — the split the ascend-by-ref work already draws. So a co-fire handler can read anything up to the root and mutate its own node, but it cannot reach the root to mutate it: `set_layer`, foregrounding, the whole existing handler set is out, because two handlers cannot both hold the path to the root and the co-fire is not the one that gets to. The one seam is the root itself, whose path is `&mut Mercury` rather than a `PathMut`; there a trait (implemented for `&mut PathMut<N, P>` and for `&mut &mut Root`) unifies the surface, the same reason `Ascend`/`AscendMut` are traits.

## What "or" really costs: the impossibility of contradiction

It is easy to read the clobber check as the thing exclusive dispatch buys. It is not. The thing it buys is that exactly one handler is the sole authority for an event: one handler is one set of effects from one author, so there is never a second effect to contradict it. The check is only the structural guard that keeps that true — no two exclusive triggers collide, so no event ever has two claimants.

Co-fire breaks the authority, which is worse than losing a check. Now the co-fire's effects and the subtree's effects both apply to one event, and whether they contradict is a semantic question about what the effects mean against the current state, not a trigger collision. The check sees only triggers, so it structurally cannot see this. Nor can it be made to by enumerating states: it already declines to check state-dependent triggers — a closure trigger is excluded from clobber detection precisely because its value is read from state at dispatch, so there is no static claim to compare — and effect-contradiction is a level past that.

So a co-fire handler appears terminal but is not. In the trigger set it looks like a terminal — one trigger, one claim — so the check either flags its intended overlap as a clobber (a false positive) or exempts it and goes blind. Neither is right, because the co-fire is not competing for the one slot; it is adding to the outcome, and "adding" is a thing the model has no vocabulary for.

The real cost, then, is that contradiction moves from statically impossible to runtime-only and unenumerable. Under exclusive dispatch two effects cannot contradict, because there is only ever one. Under co-fire they can, and whether they do depends on state that cannot be reasoned about ahead of time. A co-fire is sound only when its effect is provably independent of everything else on the path, and independence is not checkable in general — so a co-fire returns you to hand-verifying each one, which is the discipline the check existed to delete.

## Why the rearm still does not fit

Even with all of the above, the case that motivated it does not ride on it:

- The rearm's trigger is "any key," so a co-firing `AnyKey => rearm` fires on the keys that LEAVE the layer too (escape, the transitions). It arms a fresh timer, the leaving handler drops the guard, and the schedule effect self-cancels — a wasted effect in the stream on every leave. The root check (`rearm_after`) avoids this by acting only after it sees the layer stayed.
- The firing (`guard.trigger() => to_home`) reaches the root (`go_home`), so it cannot be a co-firing handler under the node-local restriction. It is exclusive, and it already works under today's dispatch.

So `AndReturnHome` wants neither an "or" bind. Both of its binds are ordinary. The rearm is exactly a non-independent co-fire: its effect, the timer schedule, is coupled to the layer's lifecycle, because a leaving handler drops the guard and the schedule self-cancels. That coupling is not incidental — it is the same reason it cannot be a bind.

## Verdict

Co-firing dispatch is buildable: partition the binds, walk the full path with an accumulator instead of short-circuiting, and restrict co-firing handlers to node-local mutation through a `&mut` path. It costs a rework of `bind`'s central `Dispatch` contract and every generated body, plus the impossibility of contradiction (above): a co-fire is sound only when its effect is provably independent of everything else on the path, and that is not checkable. It buys the ability for a node to contribute a node-local, outcome-independent effect on every event through its subtree.

Nothing in mercury needs that today. The wrapper's binds are exclusive, and the rearm is outcome-DEPENDENT (it must not fire on a leave) and root-reaching, so it is a root step regardless. This feature earns its place only when a genuine node-local per-event action appears — logging every event through a subtree, or similar — and even then the clobber cost is real. Until then, one bind kind, and the check stays sound.
