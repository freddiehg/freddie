# multiple parents, and sharing a level between hosts

Not designed. Two problems that look like one.

Background is `resolution.md`.

## Problem one: one level, several parents in ONE tree

`GmailTab` hangs under Chrome's derived level, and also somewhere else in the same tree. Same struct, same bindings, two positions.

Laserbeam already has the machinery for the PLACE case: a node with several parents declares a route enum rather than a `Path`, tested in `crates/laserbeam/tests/root_enum_multi.rs`, and `derive_support::Edge` threads a `route` through both the resolve and the dispatch descent.

Nothing wires it to a derived level. `#[derived_node(parent = InAppLayerPath)]` names ONE parent. Several would need `parent = SomeRouteEnum`, and the derive would emit a `Descend` impl over the route rather than over a concrete parent.

`Ascend` is already documented as not working here, for a different reason: a node with several parents has no unique ascent, and laserbeam says so.

## Problem two: one level, several HOSTS

A level that works in mercury's tree and in an unrelated one. This is the sub-object case: `ChromeInfo` and its bindings living in a library, and two apps hanging it wherever they like.

A handler that touches only its own node can already be generic over its parent, so one fn drives two trees:

```rust
fn shared<P>(node: Node<P, ChromeInfo>) -> Out {
    let tab = node.data.tab.clone();       // its own data. Names no host.
    ...
}
```

A handler that reaches the HOST's tree cannot. `on_chrome_r` writes the in-app layer's log, so its type names `InAppLayer`, and that is where sharing dies. It is a property of the handler, not of the node.

The bound that would save it is `Ascend`:

```rust
fn on_chrome_r<'a, P: Ascend<InAppLayerPath<'a>>>(node: Node<P, ChromeInfo>) -> Out
```

which says what the handler needs rather than where it sits, and compiles. It does not solve sharing across HOSTS, because `InAppLayerPath` is still mercury's type. Sharing across hosts needs the target to be a trait, not a path: "something I can write a log to", not "mercury's in-app layer".

That is a bigger idea and is not sketched.

## Why they are the same problem

Both are "this level does not know its parent". Multiple parents is that within a tree; multiple hosts is that across trees. Both want the derive to emit an impl that is generic over the parent rather than concrete in it.

`resolution.md` records that the generic version compiles:

```rust
impl<'a, M, P: Ascend<InAppLayerPath<'a>>> Descend<M> for Node<P, ChromeInfo> { .. }
```

and does not take it, because every derived handler then carries an `Ascend` bound, and `Ascend` does not reach through a `Node` (`ascend-through-derived.md`), so it breaks at the second derived level.

So the order is: fix `Ascend` first (Fix B in that doc, where `Path` becomes a case of `Node`), and multiple parents becomes reachable. Before that, it is not.

## Status

Blocked on `ascend-through-derived.md`. Nothing in mercury wants either today.
