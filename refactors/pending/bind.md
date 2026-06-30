# bind: the #[bind] attribute and derive

This doc is only the `#[bind(...)]` attribute and the code the derive emits per node. The runtime that consumes these bindings (accumulating the active set across the resolved path, diffing it, dispatching a fired event) is in `freddie-keys-plan.md`; precedence and clobbering are in `freddie-dispatch-precedence.md`.

bind is a crate within freddie: `bind` (the runtime traits) plus `bind_macro` (the derive, since a derive must live in a proc-macro crate).

## The attribute

`#[bind(trigger, handler, ...)]` on a node (a state struct or enum), repeatable:

```rust
#[bind(Keyboard::new('g'), on_g)]
#[bind(Keyboard::new('y'), on_y)]
struct Inner {}
```

- `trigger`: an expression that lifts into the consumer's `Trigger` enum via `Into` (`derive_more::From`). The derive wraps it with `.into()`. It is pure data, `Hash + Eq`.
- `handler`: a function, `fn(Event, Path<ThisNode, Parent>) -> Output` (below).
- A third argument is likely needed (see "The third argument").

## What the derive emits

Per node, one thing: that node's own bindings. For each `#[bind]` the derive emits a `(Trigger, thunk)` pair into the node's binding set. A node contributes only its own bindings and nothing about its place in the tree; the runtime assembles the active set by walking the resolved path.

The handler wants the typed `Path<ThisNode, Parent>`, but bindings are stored and dispatched dynamically keyed by `Trigger`, so each thunk is the erasure boundary: given `&mut Root` and the fired event, it reconstructs the typed path to this node (the descent rayban already walks) and calls the typed handler. The thunk is `Fn(&mut Root, Event) -> Output`.

The trigger is an ordinary expression of the `Trigger` type, not restricted to a literal, evaluated when the binding set is built. It is owned and moved in, so it needs no `'static` bound.

## The handler

`fn(Event, Path<ThisNode, Parent>) -> Output`. It mutates state through the path (`get_mut`, `into_parent`, `get_root`) and returns effect data; it performs no I/O. `bind` is generic over `Event` and `Output` (mercury uses `MercuryEvent` and `Option<Vec<MercuryEffect>>`); it passes them through and does not interpret them.

## The third argument

`#[bind(trigger, handler)]` probably needs a third parameter. Candidates:

- A description: a human-readable label for the binding, for a which-key-style overlay and generated documentation of the state space. Independent of v1 scoping, and the project already wants overlays and docs (the type-level state enumeration in `rayban-missing-features.md`).
- A clobber or precedence policy: whether this binding is clobberable, or a priority, per `freddie-dispatch-precedence.md`. v1 is uniformly non-clobberable, so this is not needed until that lands.
- A guard: a predicate gating the binding beyond the trigger. Probably unnecessary, since conditions are better expressed as state (the active node already encodes the condition).

Leaning toward the description. Open: which one, and whether it is positional or named (`#[bind(trigger, handler, desc = "..")]`).
