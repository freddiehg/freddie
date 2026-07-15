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

## Projecting into `data` is allowed, because otherwise nothing composes

A `#[resolve_into]` child of a derived level compiles, gets a real `Path` and a real `get_mut()`, and writes into `data`, which the derived child fn built and which dies with the dispatch.

The derive could reject it: it sees `#[derived_node]` and `#[resolve_into]` on the same struct. Do not.

Rejecting it would mean a node type can only be used in one position. A subtree that is legal under a place becomes illegal under a derived level, so every node has to know whether its ancestors are derived, and a subtree can no longer be lifted from one part of the tree to another. Composition is the point; a node should work wherever it is hung.

It is also not a new rule. `data` is owned by the node and dies with the dispatch, at every level and through any number of projections into it. Saying that once covers this case. A derived level's `data` may have sub-structure with its own bindings, whose handlers read that sub-structure and write the tree through `parent`, and that is a legitimate program.

### The same node in both positions

The case a compile error would foreclose: `GmailTab` appearing TWICE in one tree, once as a real node in a persisted branch and once as the `data` of a derived level. Same struct, two positions, one set of bindings.

Whether that works is open and not proven. It is multi-parent, which laserbeam already has route enums for (`crates/laserbeam/tests/root_enum_multi.rs`), but the two positions differ in more than the parent: a real node's handler is handed `Node<GmailTabPath, ()>` and a derived one's is handed `Node<Parent, GmailTab>`, so one handler cannot currently serve both.

It is not a reason to allow the projection. It is a reason not to forbid it before knowing.
