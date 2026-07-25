# Invalidation granularity

Thoughts on debt, not a plan. Background: `refactors/past/invalidation.md` (the model), `mercury-post-patterns.md` (the consumer that hit it; its concrete case, the overlay toggle, is resolved there by binding the key at the node that owns the field, so nothing in it gates that doc anymore). The general hole remains: it reopens the day a writer cannot bind at its state's owner.

## The problem

Invalidation is path-granular; a write is field-granular. The type system's only lever for "I need `&mut` an ancestor" is consuming the path to it, and the `Completed` that comes back speaks for the whole subtree below the stop: everything is invalidated, one bit for the entire tree.

`toggle_overlay` was the concrete case. `overlay: Option<TimerGuard>` lives on the root, and today no path runs through it, so a leaf handler writing it invalidates nothing that exists — but it must consume to the root to reach it, and the returned leave claims everything below the root died; a deadline post would read an o-press as a leave and cancel the return-home timer for a layer the user is still in. The escape that worked there — bind the key at the node that owns the field, making the write own-node — only exists because the root can bind `o`; it is not available to a writer whose trigger belongs to a deeper node.

The reverse direction is also unrepresentable. Once the overlay grows its own bound subtree (an overlay with keys while it is open, `multiple-children.md`), writing it should invalidate overlay paths and not the layer path, and the one-bit answer cannot say that either.

What is missing is a way to say which part of the tree a write touched.

## Rejected reaches

- `ancestor_mut` (mutate an ancestor from a standing path): interior mutability in a static costume, and invalidation cannot price it — the write claims to invalidate nothing when the type system has no way to check what it touched.
- Write-requests re-entering as events: re-dispatching to reach state is working around the same hole, makes dataflow hard to reason about, and runs into the feedback rule below.

## Candidate shapes

- The root is a tuple: several sibling trees under one dispatch, a path addressing one component, `Completed<P>` speaking only for its component. Writing another component invalidates nothing on this path, by type. The overlay becomes a second component, and its future bindings hang under it.
- Invalidatable and non-invalidatable state: the root partitions into tree state (paths run through it; writes invalidate) and plain state (no paths; any handler writes it through a handle that does not end the path). The line is per field, and a field crosses it the day it grows bindings — the overlay sits on the plain side today and would have to cross for `multiple-children.md`.

Both need the same decision made precisely: what `Completed<P>` means when state outside `P`'s chain changed. Not designed.

## The feedback rule

No effect handler may immediately invoke an event. An effect that synchronously enqueues an event is an unbounded loop one bind away, and it launders state-reach problems through the event queue. The rule is currently discipline; it is hard to enforce statically. A runtime enforcement sketch: the event channel is closed to same-thread enqueues while the current dispatch's effects are being performed (a flag on the loop), and a send inside that window is a loud error. Asynchronous arrivals — timers firing later, watcher threads, the socket — are unaffected; only the synchronous perform path is fenced. Low priority; recorded so the rule is written down.
