# why a handler takes the event by reference

A handler is `fn handler(ev: &KeyEvent, node: Node<...>) -> Vec<MercuryEffect>`. The event comes in by shared reference; the state path (`node`) comes in by value as an `&mut` path into the tree. This doc is why the event is a `&` and not owned, and what it would take to change that.

## The reason: dispatch offers one event to many matchers

Dispatch is a walk, not a call. One event is shown to a chain of candidate matchers, and only the one that claims it runs its handler. The signatures thread the event as a borrow through that whole walk:

- `EventTrigger::is_matching(&self, event: &Self::Event) -> bool` (`bind/src/lib.rs:191`) is the selection step, by reference.
- `Descend::dispatch(self, event: &M::Event) -> ControlFlow<M::Output, Self::Parent>` (`bind/src/lib.rs:208`) and `Dispatch::dispatch(path, event: &M::Event)` (`bind/src/lib.rs:235`) are the descent. Each node tries its active child first, then its own binds; on a miss it hands the PARENT back, and the same event is dispatched against the ancestor.

So a single physical event is offered to a chain of `is_matching` checks, leafward first and then up via `into_parent`, until a node claims it. You cannot move a value into a matcher that might decline it. Ownership could only go to the winner, but the winner is not known until the event has been borrowed for every `is_matching` on the way there. That is the structural reason it is a `&`.

## The event is read-only fan-in

A handler's outputs are its returned `Vec<MercuryEffect>` and its mutations through `node` (the `&mut` path). The event is an input it reads, not something it owns or stores.

Most handlers ignore it: they bind `_ev` (`home.rs`, `nav.rs`, `resize.rs`, `app.rs` all take `_ev: &KeyEvent`). The two that read it touch only a field or two:

- `modify_held_and_pass_through` (`mercury/src/handlers/typing.rs:37`) reads `ev.key` and `ev.press` to update `held.cmd`, then emits.
- `pass_through` (`mercury/src/handlers/toggle.rs:26`) reads the event to re-emit it.

A shared borrow is exactly the access they need. The unified event is even extracted as a reference: the trigger does `TryFrom<&Event> for &SourceEvent` (the type match described at `bind/src/lib.rs:183`), so the borrow flows through the type match as a `&`.

## Could it take ownership?

- Not under the current dispatch shape. The generic signatures thread `event: &M::Event` uniformly through both selection and execution, and selection needs the event still shared while `is_matching` runs on each trigger. A by-value handler contract is incompatible with "offer to N triggers, one runs."
- A redesign could do it: `is_matching` on a borrow, then move the owned event into the single arm that fires. Dispatch runs exactly one handler per event (leafward), so there is a well-defined winner to move into.

What ownership would buy, and why it is not taken:

- The only saving is a clone. `KeyEvent` is `Clone` but not `Copy` (`freddie_keys/src/lib.rs:132`, deriving `Clone, PartialEq, Eq, Debug`). `modify_held_and_pass_through` builds `MercuryEffect::Emit(ev.clone())` and `pass_through` does the same; an owned event would let those two `Emit(ev)` without the clone.
- It is not worth it. `KeyEvent` is a `Key` plus a `PressType`, two small enums, so the clone is a few bytes in exactly the two emit handlers. Every other handler ignores the event, and a shared ref they drop for free is a lighter contract than owning-and-dropping.
- Moving the source event out would also mean owning and destructuring the unified event enum instead of the current `&`-conversion (`TryFrom<&Event> for &SourceEvent`), more dispatch machinery for a clone that costs nothing.

So the event is borrowed because dispatch offers one event to many matchers and only a borrow can be shared across that walk. Ownership is possible with a dispatch redesign that moves the event into the matched arm; it would remove one small clone in two handlers and is not worth the added complexity.
