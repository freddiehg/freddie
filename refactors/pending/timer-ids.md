# one timer event, many timers

Punted, not settled. `jk-timeout.md` does not depend on this and adds a `JkTimeout` type of its own; this is the cleanup that would remove both it and `LayerTimeout`.

## The problem

Every timer mercury owns costs a type: `LayerTimeout` is a struct, a `MercuryTrigger` variant, a `MercuryEvent` variant, and a `self_trigger!` impl, all to say "the layer's idle timer went off". The `jk` window will cost the same again, and the next timer after that.

Nothing about any of it is per-timer except which timer fired.

## Two candidates, and the open question

An enum registry: one `TimerFired { id: TimerId }` event and a `Timer(TimerId)` trigger, with `TimerId` naming each timer. It reads well in the log, which matters because mercury reconstructs a run from one record per dispatched event, and the ids keep the duplicate-trigger check meaningful, since `Timer(ReturnHome)` and `Timer(JkWindow)` are different trigger values.

Against it: `TimerId` becomes a registry of every timer in the app, sitting in `sources.rs`, so adding a timer still means editing a shared enum far from the code that arms it. That is the same coupling as the bespoke types, one variant cheaper. It moves the complexity rather than removing it.

A per-armed identity: the id is generated when the timer is armed, carried by the guard, and the binding reads it off the node that holds the guard. Adding a timer touches nothing shared.

What makes this possible, and what I got wrong the first time: a trigger does NOT have to be a compile-time constant. `bind_macro` parses it as a `syn::Expr` and emits `let trigger = #trigger;` INSIDE the dispatch body, where `path` is still in scope and has not yet been moved into the handler. So a trigger can read node state — `Timer(path.typing_state.jk_timer_id)` — and a generated id works.

It also fixes something the enum cannot. With a registry, a stale firing from a timer we already abandoned still matches the binding, and only the guard's cancellation stops it acting; today's test comment leans on "a stale timeout is harmless" because home re-entering home does nothing. With a per-armed id, a stale firing matches no live binding, so acting on it is structurally impossible.

Against it: an opaque id in the log where a name would be, unless the id carries something readable.

Unverified: I did not get a clean compile of a trigger reading `path`. The experiment failed on an unrelated mistake in the harness rather than on the trigger itself, so the codegen argument stands but has not been demonstrated. Do that first.

## What is decided

Nothing yet. Settle the id question before writing the changes out.
