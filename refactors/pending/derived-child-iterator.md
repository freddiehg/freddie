# derived child, several at once

Not designed. Recorded as a possibility.

Background is `resolution.md`: a derived child fn is `fn(&Parent) -> Option<Data>`, and a node descends into at most one child.

## The unification

`Option<T>` IS an `IntoIterator<Item = T>`. So one signature covers zero, one, and many, and the shipping design is the arity-0-or-1 case of it with no special-casing anywhere.

```rust
fn f(&Parent) -> impl IntoIterator<Item = Data>
```

`None` is the empty iterator. `Some(d)` is the one-element iterator. A browser's tabs, a tmux session's windows, a set of panes are the rest.

The derive would not change shape. Today it emits a `match` on the `Option`; it would emit a loop over the iterator.

## What it collides with

Dispatch holds one `&mut` into the tree. A node builds a child by moving its own parent into it, so two live children would need two `&mut Root` at once, which does not exist.

So the children are visited ONE AT A TIME: build child `i`, dispatch, take the parent back on a miss, build child `i + 1`. Each step consumes and returns the parent, which is the `ControlFlow` shape the current signature deleted. It comes back, but inside the loop and inside the framework, never in a user's signature.

## What it does not answer

Which child gets the event. With one child, "the leaf wins" is the whole rule. With several, the ITERATION ORDER becomes the rule, which is the same trap as the pre-`no-clobber` scan: two children claiming one trigger, and whichever the iterator yields first silently wins.

`no-clobber.md` already has the machinery for it. Several children each claiming distinct triggers is fine; two claiming the same one is a clobber, and `accumulate` catches it by walking the same iterator.

## Where it might actually pay

A bind that fans out over an index. `Key::Num1.down()` through `Key::Num0.down()`, each doing the same thing to a different tmux window, is ten binds and ten handlers in `crates/mercury/src/lib.rs` today. One derived child per index, each with `data: { index }`, would make it one bind on one node.

That is a different motivation from "several children exist", and it may be the stronger one.

## The alternative it competes with

If several children exist but only ONE is active, that is `Option<Data>` and always was. The derived child fn selects, and the collection stays as state in the tree.

The iterator earns its place only when several children are simultaneously live and each binds different keys. Whether mercury ever wants that is unknown.

## Status

Not needed. `Option<Data>` is the arity that ships, and it generalises to `IntoIterator` later without breaking anything.
