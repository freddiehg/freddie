# timer events

A timer is a source, not an effect. It fires an event, that event dispatches like any other, and a handler mutates state through the path it holds.

Saying it the other way round is the mistake worth avoiding. If scheduling were an effect, a handler would return `ScheduleTimer { after, id }`, something would have to remember the id, and a `TimerFired(id)` event would have to be correlated back to the state that asked for it. Every one of those pieces exists only because the timer was modeled in the wrong direction.

## Timers are stateful, so the state holds them

A running timer has state: a deadline, and the fact that it is running at all. The design says all state lives in one tree. So the timer lives in the node:

```rust
struct SomeLayer {
    timeout: Timer, // dropping it cancels
}
```

`Timer` is an RAII handle. Constructing it arms; dropping it cancels. Nothing else is required, and in particular no registry, no ids, and no diff.

This falls out of laserbeam rather than being bolted onto it. A transition is `*layer.get_mut() = Layer::Home(HomeLayer::new(ctx))`, which drops whatever variant was there, which drops its `Timer`, which cancels it. Leaving a state cancels its timers because leaving a state drops it. There is no way to leak a timer belonging to a state you are no longer in, and no code enforces that, because it is not a rule. It is ownership.

Three things fall out for free.

Re-entering a state restarts its timer, because a fresh node constructs a fresh `Timer`. mercury re-enters `Home` on every `escape`.

Carrying a timer across a transition is moving the field into the new node. `overall-plan.md` already said carry-over is possible because the cursor owns the data.

A repeat timer is a handler that advances its own deadline: the fire event dispatches, the handler replaces the node's `Timer` with a new one. Which is exactly "before-event and after-event are two distinct states."

It is the same shape as `Interceptor`, `Watcher`, and `Stopper`. Drop deregisters.

## Dropping cancels, but a fired event is already gone

Concretely, `Nav` holds a ten-second timer that takes us home.

```rust
struct NavLayer {
    idle: Timer, // ten seconds, fires Timeout
}
```

Press `c` at three seconds and the handler swaps the layer to `Home`. The old `NavLayer` is dropped, its `Timer` with it, the task is aborted, and it never fires. Let it run instead, and at ten seconds it fires, the `Timeout` event dispatches, `Nav`'s handler swaps to `Home`, and the same drop happens with nothing left to cancel. Either way there is nothing to clean up and nothing to remember.

There is one gap, and `Drop` cannot close it. The timer fires at `t = 10.000` and pushes its event into the channel. At `t = 10.001` a key event that was already ahead of it in the queue transitions `Nav` to `Home`. Dropping the `Timer` aborts the task, but the event has been sent. Drop cannot un-send it, so `Timeout` dispatches against `Home`.

Today that is harmless: `Home` binds nothing for it and dispatch returns `None`. It stops being harmless as soon as two states bind a timeout, because a dead `Nav` timer would fire `Home`'s handler.

The fix is the same ownership. `Timer` holds an `Arc<()>` and the fired event carries a `Weak<()>`. Dropping the node drops the `Arc`, the `Weak` is dead, and the event loop discards the event before dispatch. A cancelled timer's event cannot reach the model, and there is still no id anywhere.

## Owned sources and shared sources

A timer is not the general case. It is one of two kinds, and conflating them is easy.

An **owned** source is one the node wants its own instance of, with its own parameters. A timer is owned: `Nav`'s ten seconds is not `Home`'s ten seconds, they are different timers that happen to share a duration. Cardinality is one per node, lifetime is the node's, and `Drop` cancels. Nothing outside the tree.

A **shared** source is one instance for the process, of which many nodes take a view. The keyboard is shared. `Mercury` binds `escape`, `Layer` binds `tab`, `Nav` binds `c`: three nodes, three subscriptions, one tap, and not one of them owns it. The node holds nothing; the binding is the subscription.

The test is whether the node wants its own instance or a slice of a common one. `Nav` wants its own deadline. `Nav` does not want its own keyboard.

`bind::accumulate` is for the shared kind. Its union over the active path is the set of live subscriptions, and diffing it across a dispatch says when to register a shared source and when to tear it down: register while at least one active node subscribes, deregister when none do. That is why nothing consumes it today. The tap delivers every key whether or not anything is bound, so registration is a no-op, and the union is never interesting. It becomes interesting for a source where registering costs something, or where the set of subscriptions decides what to ask the OS for.

So the two mechanisms are not rivals and neither subsumes the other. Ownership handles owned sources. The diff handles shared ones. A timer is the easy case precisely because it is owned and needs no OS registration at all.

Where the foreground watcher falls is not obvious. mercury registers one `Watcher` for the process lifetime and every state sees its events, which is the shared shape. A state that wanted app activations only while it was active could hold its own `Watcher`, since `Drop` deregisters, which is the owned shape. Both work, and nothing has needed to choose.

## What this costs

The state tree stops being pure data. A `Timer` holds a task handle, so a node holding one is not `Default`, not `Clone`, and not constructible without a runtime and a way to send the fire event back. `overall-plan.md` anticipated this: "An optional context struct lets actions close over external handles; pass it as a second argument when present." That context is what a handler needs in order to construct a `Timer`.

mercury derives `Debug` on every state node in order to log the whole tree on each dispatch, so `Timer` has to be `Debug`, which is a manual impl printing the deadline rather than the task handle.

Tests construct states directly and dispatch against them with no runtime. `Mercury::default()` cannot arm a timer. Either `Timer` has a null construction for tests, or nodes with timers are only built through handlers that take a context.

## The road not taken

The alternative keeps the state pure and makes the timer a trigger. `bind::accumulate` returns the active trigger set, an outer handler diffs it across a dispatch, and arms or cancels timers as triggers appear and disappear. This is the mechanism `overall-plan.md` described:

> One outer handler owns registration. It receives the accumulated `Trigger` diff and routes each variant to its OS mechanism.

It works, and it is worse. `After(Duration)` is not an identity: two states binding `After(500ms)` are the same trigger, so a transition between them arms nothing, two on one active path are a `DuplicateTrigger` error, and re-entering a state changes nothing in the set. The fix is a trigger carrying the deadline, `At(Instant)`, which is `Eq + Hash + Copy` and therefore a legal trigger. Then re-entry produces a new `Instant` and the diff notices.

But the running timer's task handle still has to live somewhere, and that somewhere is a `HashMap<Trigger, JoinHandle>` in the registration handler. That is mutable state outside the tree, which is the thing the single-root premise exists to prevent. Ownership in the node gets the same behavior with nothing outside, because a timer is an owned source.

So timers do not force `accumulate`'s diff to be built. Nothing consumes `accumulate` today, and a timer is not the thing that changes that. A shared source whose registration costs something would be.

## What wants timers

Tap versus hold. Explicitly not a primitive. `escape` tapped goes home; `escape` held enters a layer. Two states and a timer between them.

Keyboard-mouse mode. Continuous pointer motion needs a repeating timer feeding events while a key is held, not one event per keypress.

Auto-hiding an overlay. voicemode's `showBrief(layer)` flashes the layer name and clears it. That is a state with a timer.

Debouncing. Rapid app switches and display reconfiguration both produce bursts; both `refactors/past/foreground-events.md` and `display-events.md` name debouncing as open. A node that holds a `Timer` and replaces it on every arriving event is a debounce, and it keeps the debounce in the model rather than in the source crates.

## What mercury does today

There is exactly one timer, and it cheats. `spawn_killswitch` sleeps and then sends `MercuryEffect::Kill` straight into the effect channel, bypassing the model. The state tree never sees it. That is fine for a dev safety net, and it is not the pattern.

## Open questions

- What does `Timer::new` take? A duration, a context holding the event sender, and the event to fire. The event has to be `M::Event`, which makes `Timer` generic over the marker, or the context concrete per consumer.
- One-shot and repeat as one type or two? A repeat is a handler replacing its own `Timer`, so possibly only one type is needed.
- How do tests construct a node holding a `Timer` with no runtime?
- Where the timer runs. `tokio::time` on the worker thread is already there. A `CFRunLoopTimer` on main would deliver where AppKit callbacks deliver, and nothing wants that.
- Interaction with `prioritization.md`. A fast repeat timer feeding the same queue as the keyboard can starve typing, which is the case that doc was written for.
- Is the `Weak<()>` check the right place to discard a cancelled timer's in-flight event, and does it belong in the event loop or in dispatch?
- Is the foreground watcher owned or shared? It is registered once for the process today, which is shared, and `Watcher::drop` deregisters, which would let a node own one.
- What shared source has a registration cost high enough to make `accumulate`'s diff worth building?
