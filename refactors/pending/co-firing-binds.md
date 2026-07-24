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

## The handler restriction

Phase 1 runs before phase 2, and phase 2 (the child descent) needs the path. So a co-firing handler must not consume the path. Every mercury handler today consumes it — they take `Node<Path, ()>` and `ascend_mut()` to the root to mutate root state. A co-firing handler cannot: it takes the node by mutable borrow and mutates only the node's own data:

```rust
// exclusive handler, today's shape: consumes the path, ascends to the root
fn to_home<'a, E, P: AscendMut<MercuryPath<'a>>>(ev: &E, node: Node<P, ()>) -> Vec<Effect>;

// co-firing handler: borrows this node, mutates it, returns; the path lives on to descend
fn touch(ev: &KeyEvent, node: &mut AndReturnHome) -> Vec<Effect>;
```

So co-firing binds can only do node-local work. Anything that reaches the root (`set_layer`, foregrounding, the whole existing handler set) is out. This is a hard boundary, not a nicety: two handlers cannot both hold the path to the root, so the one that co-fires cannot be one of them.

## The check, and why "or" costs the clobber guarantee

The check collects every live node's triggers on the active path and errors on a duplicate. That is what makes the tree safe to reason about, and what makes an exclusive wrapper free: its triggers are disjoint from the inner's, so no overlap, no error.

A co-firing bind deliberately overlaps — firing alongside a child binding is its purpose. So a co-firing trigger that collides with another binding is legal, while an exclusive one is a bug, and the check sees only trigger values. It has to be told which triggers are co-firing (exempt from the collision rule) and which are exclusive (must be unique). Once a trigger is exempt, a genuine contradiction under it is undetectable — you asserted the collision was intentional. So introducing co-firing binds forfeits, for those triggers, the one guarantee the check exists to give.

## Why the rearm still does not fit

Even with all of the above, the case that motivated it does not ride on it:

- The rearm's trigger is "any key," so a co-firing `AnyKey => rearm` fires on the keys that LEAVE the layer too (escape, the transitions). It arms a fresh timer, the leaving handler drops the guard, and the schedule effect self-cancels — a wasted effect in the stream on every leave. The root check (`rearm_after`) avoids this by acting only after it sees the layer stayed.
- The firing (`guard.trigger() => to_home`) reaches the root (`go_home`), so it cannot be a co-firing handler under the node-local restriction. It is exclusive, and it already works under today's dispatch.

So `AndReturnHome` wants neither an "or" bind. Both of its binds are ordinary.

## Verdict

Co-firing dispatch is buildable: partition the binds, walk the full path with an accumulator instead of short-circuiting, and restrict co-firing handlers to node-local mutation. It costs a rework of `bind`'s central `Dispatch` contract and every generated body, plus the clobber guarantee for co-firing triggers. It buys the ability for a node to contribute a node-local, outcome-independent effect on every event through its subtree.

Nothing in mercury needs that today. The wrapper's binds are exclusive, and the rearm is outcome-DEPENDENT (it must not fire on a leave) and root-reaching, so it is a root step regardless. This feature earns its place only when a genuine node-local per-event action appears — logging every event through a subtree, or similar — and even then the clobber cost is real. Until then, one bind kind, and the check stays sound.
