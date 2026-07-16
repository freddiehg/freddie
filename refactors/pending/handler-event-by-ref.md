# why a handler takes the event by reference

A handler is `fn handler(ev: &KeyEvent, node: Node<...>) -> Vec<MercuryEffect>`. The event comes in by shared reference; the state path (`node`) comes in by value as an `&mut` path into the tree. This doc is why the event is a `&` and not owned, and what it would take to change that.

## The reason: dispatch offers one event to many matchers

Dispatch is a walk, not a call. One event is shown to a chain of candidate matchers, and only the one that claims it runs its handler. The signatures thread the event as a borrow through that whole walk:

- `EventTrigger::is_matching(&self, event: &Self::Event) -> bool` (`bind/src/lib.rs:191`) is the selection step, by reference.
- `Descend::dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>` (`bind/src/lib.rs:208`) and `Dispatch::dispatch(path, event: &M::Event)` (`bind/src/lib.rs:235`) are the descent. Each node tries its active child first, then its own binds; on a miss it hands the PARENT back, and the same event is dispatched against the ancestor.

So a single physical event is offered to a chain of `is_matching` checks, leafward first and then up via `into_parent`, until a node claims it. You cannot move a value into a matcher that might decline it. Ownership could only go to the winner, but the winner is not known until the event has been borrowed for every `is_matching` on the way there. That is the structural reason it is a `&`.

## The event is read-only fan-in

A handler's outputs are its returned `Vec<MercuryEffect>` and its mutations through `node` (the `&mut` path). The event is an input it reads, not something it owns or stores.

Most handlers ignore it: the layer transitions and app commands bind `_ev` (`home.rs`, `nav.rs`, `resize.rs`, `app.rs`). The ones that read it are the root passthrough and typing's escape, and they touch only fields:

- `on_modifier` and `maybe_pass_through` (`mercury/src/handlers/root.rs`) read `ev.key`, `ev.press`, and `ev.flags` to re-emit the key; `on_modifier` also feeds `ev` to `HeldModifiers::apply` to track the modifier.
- `maybe_go_home` (`mercury/src/handlers/typing.rs`) reads them to re-emit a plain escape.

A shared borrow is exactly the access they need. The unified event is even extracted as a reference: the trigger does `TryFrom<&Event> for &SourceEvent` (the type match described at `bind/src/lib.rs:183`), so the borrow flows through the type match as a `&`.

## Could it take ownership?

- Not under the current dispatch shape. The generic signatures thread `event: &M::Event` uniformly through both selection and execution, and selection needs the event still shared while `is_matching` runs on each trigger. A by-value handler contract is incompatible with "offer to N triggers, one runs."
- A redesign could do it: `is_matching` on a borrow, then move the owned event into the single arm that fires. Dispatch runs exactly one handler per event (leafward), so there is a well-defined winner to move into.

What ownership would buy, and why it is not taken:

- There is not even a clone to save. `KeyEvent` is `Clone` (a `Key`, a `PressType`, and `ModifierFlags`, all `Copy`, though the struct is not `#[derive(Copy)]`), and no handler clones it: the passthrough handlers read `ev.key`/`ev.press`/`ev.flags` and build a fresh event through the `emit` helper. An owned event would let a straight passthrough do `Emit(ev)` (a move) instead of reconstructing, which copies the same few bytes either way.
- It is not worth it. Every other handler ignores the event, and a shared ref they drop for free is a lighter contract than owning-and-dropping.
- Moving the source event out would also mean owning and destructuring the unified event enum instead of the current `&`-conversion (`TryFrom<&Event> for &SourceEvent`), more dispatch machinery for a move that costs nothing.

So the event is borrowed because dispatch offers one event to many matchers and only a borrow can be shared across that walk. Ownership is possible with a dispatch redesign that moves the event into the matched arm, but with no clone to remove it buys nothing and is not worth the complexity.
