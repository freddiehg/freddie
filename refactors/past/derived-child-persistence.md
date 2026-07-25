# derived-child persistence

DO NOT DO. Rejected. A persisting derived child (constructor-on-enter, destructor-on-leave) means storing it, which means it is in the tree, which means it can go stale, which is the bug the derived-child design deletes.

Rejected. Recorded so it does not get re-proposed.

Background is `resolution.md`: a derived child fn builds a derived level's `data` from root state on every dispatch, and that data is owned by the node, so writing it changes nothing outside the dispatch.

## The proposal

Make the derived child fn a constructor, run when the tree enters the state, and add a destructor, run when it leaves. The data would then live across dispatches, and a `#[resolve_into]` child of a derived level would write to something that survives.

## Why not

Persisting means storing. Storing means it is in the tree. The moment it is in the tree it can disagree with the thing it was derived from, which is `AppLayer::for_root` and the bug `resolution.md` exists to delete.

The derived child fn's whole value is that it is rebuilt from root state every dispatch, so there is nothing to invalidate. Give it a lifetime longer than one dispatch and you owe an answer to "what if `root.app` changed" and "what if the tab changed." The answers are:

- Re-run it. Which is what we already do, so the cache bought nothing.
- Track what it depends on. Which is a reactive system.

The destructor is worse than the constructor. Running it means detecting that the tree LEFT the state, which means comparing this dispatch's resolution against the last one, which means storing the last one, which is more derived state with the same problem. And a handler that mutates `root.app` mid-dispatch would leave a live node that is already dead.

## Dependency tracking is possible, and is a can of worms

The mechanism exists. A derived child fn is `fn(&Parent) -> Option<Data>`, so it can only read, and the path API already separates reads from writes:

```rust
path.parent()      // &Parent      a READ.  Available to a derived child fn.
path.get_mut()     // &mut Node    a WRITE. Needs &mut, so a derived child fn cannot.
```

So laserbeam could record which nodes a derived child fn read while building its data, and invalidate the memo when one of those nodes is written. That is a dependency graph, and it is sound.

The shared reference is also why a derived child fn cannot materialize its data into the tree and project to it, which would be the other way to make it persist.

What it costs:

- A dependency set per memoized node, stored somewhere, and invalidated on every `get_mut()` anywhere in the tree.
- The memo itself is derived state that has to be typed and stored, and the derive cannot name the derived child fn's return type. The user would have to declare the storage, at which point it is a field, at which point it is `#[resolve_into]` and not a derived child fn.
- Reads through `parent()` are coarse: reading `path.parent().app` records a dependency on the whole parent node, not on `app`. Any write to that node invalidates.

Not worth it. Rebuilding is one clone.

## The rule that falls out

Two options, and no third.

- The data should persist: put it in the tree as a real field, reach it with `#[resolve_into]`, and own the invalidation.
- The data should always be fresh: build it in a derived child fn, and own nothing.

The question "do you own the invalidation" is the only question, and it has exactly these two answers.

## Projecting into `data`: superseded (2026-07-25); the derive errors

An earlier revision of this section permitted a `#[resolve_into]` child of a derived level (bindings on sub-structure of `data`, handlers writing the tree through `parent`) and told the derive not to reject it. That permission never shipped and is withdrawn.

- It never shipped: `derived_node_impl` builds its descent only from `#[derived_child]` and never consults `find_resolve_into`, so the attribute was accepted and silently ignored — no descent, no dispatch of the child's binds, no diagnostic, and the child's triggers invisible to accumulate. A silent "this is never reachable."
- It is now unimplementable: under invalidation (`refactors/pending/derived-levels.md`), descending would mean folding the child's `Completed<PathMut<Sub, Node<..>>>`, which requires `Node: Above`, which the flattening design refuses. A `Node` is not a path, and everything below the last real place ascends at that place.

So `derived_node_impl` and `derived_enum_node_impl` error on any `#[resolve_into]` field (`derived-levels.md`, change 5's macro deltas), and the error states the two-options rule above:

```text
a derived level cannot have a `#[resolve_into]` child: its `data` dies with the
dispatch. Persist the state in the tree at a real place the derived level reads,
or hang a fresh level with `#[derived_child]`.
```

The composition case the old permission protected — one struct hung both as a real node in a persisted branch and as a derived level's `data`, with one set of bindings — is foreclosed by the leave types regardless of the derive: the two positions' dispatches would return `Completed` of different shapes, and the derived position has none. The error costs nothing the permission was still buying.
