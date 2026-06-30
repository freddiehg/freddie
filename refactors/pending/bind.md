# bind: the #[bind] attribute and derive

This doc is only the `#[bind(...)]` attribute and the code the derive emits per node. The runtime that consumes these bindings (accumulating the active set across the resolved path, diffing it, dispatching a fired event) is in `freddie-keys-plan.md`; precedence and clobbering are in `freddie-dispatch-precedence.md`.

bind is a crate within freddie: `bind` (the runtime traits) plus `bind_macro` (the derive, since a derive must live in a proc-macro crate).

## The attribute

`#[bind(trigger, handler)]` on a node (a state struct or enum), repeatable:

```rust
#[bind(Keyboard::new('g'), on_g)]
#[bind(Keyboard::new('y'), on_y)]
struct Inner {}
```

- `trigger`: an expression that lifts into the consumer's `Trigger` enum via `Into` (`derive_more::From`). The derive wraps it with `.into()`. It is pure data, `Hash + Eq`.
- `handler`: a function, `fn(Event, Path<ThisNode, Parent>) -> Output` (below).

## What the derive emits

Per node, the macro emits two things over that node's `#[bind]`s, both generic over `T: From<each source the node uses>` (so the per-source trigger values lift into one enum; see below):

- the node's triggers, each lifted with `.into()` to `T`, for the active-set accumulation (keys only);
- a dispatch hook: given a fired `T` and the node's live path, if one of the node's triggers matches, it calls that handler with the path and returns the `Output`.

Because dispatch runs the handler while the node's path is live, there is no stored handler map, no type erasure, and no path reconstruction. A node contributes only its own bindings; walking the active path to accumulate the set and to find the handler is the runtime's job (`freddie-keys-plan.md`).

The trigger is an ordinary expression, not restricted to a literal, evaluated when the set is built; it is owned and moved in, so it needs no `'static` bound.

## The handler

`fn(Event, Path<ThisNode, Parent>) -> Output`. It mutates state through the path (`get_mut`, `into_parent`, `get_root`) and returns effect data; it performs no I/O. `bind` is generic over `Event` and `Output` (mercury uses `MercuryEvent` and `Option<Vec<MercuryEffect>>`); it passes them through and does not interpret them.

## Combining per-source triggers into one enum

`Keyboard::from('g')` and `Foreground::from("chrome")` are different types, so a node that binds across sources needs them in one `Trigger` enum to accumulate. The macro emits `.into()` on each trigger expression and makes the node's generated code generic over `T: From<Keyboard> + From<Foreground> + ..` (one `From` bound per source the node uses). The consumer's `Trigger` enum (with `derive_more::From`) is the concrete `T`. The enum is named once, by the consumer, and carried in through the `From` bounds; it is not a third argument on each `#[bind]`.
