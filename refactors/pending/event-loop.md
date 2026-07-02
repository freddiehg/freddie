# the event loop

Dispatch turns one event into effects. The loop feeds it events and performs the effects. The effects and the events they cause are decoupled: opening Chrome is one action, and Chrome coming to the foreground is a separate event that arrives later. A handler does not return the follow-up event; it triggers the effect, and the resulting event is delivered like any other input.

## The queue

Events land in one queue. The loop drains it, and everything that produces events pushes to it:

- the keyboard source,
- the foreground watcher,
- the effect handler, for an event it knows synchronously.

The loop pops an event, dispatches it, and hands the output to the handler, which performs the effects and pushes whatever events it can. When the queue is empty the loop waits for the next event.

Opening Chrome: the handler tells the OS to open Chrome and returns. The foreground watcher, seeing Chrome come up, pushes `Foreground(Chrome)`. Nothing synchronously ties the two.

## Single thread vs threads

Single-threaded: the OS delivers input on one thread. A `CGEventTap` and the workspace foreground notifications are run-loop sources, so the queue is a plain local `VecDeque` pushed and popped on that one thread. Events stay local, so they need neither `Send` nor `'static`.

Multi-threaded: the queue is a channel (`mpsc`). Sources on other threads hold `Sender`s and the loop holds the `Receiver`. Events cross threads, so they must be `Send + 'static`.

The `'static` is only forced by threads. A remapper's inputs are already run-loop sources on one thread, so the single-threaded queue avoids it, and it is the natural macOS shape.

## Shape

There is no generic `run` in freddie. The loop is a handful of lines, the queue and the wait-when-empty differ per consumer, and dispatching a generic root needs an awkward `Resolve<Path<'a> = &'a mut N>` bound that an inline loop over a concrete root avoids. So each consumer writes its own loop; `bind::dispatch` (event to output) and `bind::accumulate` (the active triggers) are the pieces.

The loop pops an event, dispatches it, and for each effect lets a handler perform it and push any synchronously known follow-up onto the queue:

```rust
let mut queue: VecDeque<Event> = ...;
while let Some(event) = queue.pop_front() {
    if let Some(output) = state.handle(&event) {
        for effect in output {
            // perform the effect; push a follow-up event if one is known now
        }
    }
}
```

The tests use exactly this drain-and-re-queue form, with no waiting: a `Foreground` effect pushes its follow-up onto the same `VecDeque`. The real CLI uses the same shape, but its queue is fed by the OS listeners and it waits when the queue is empty.

## Tests

The per-event tests dispatch one event and assert the output; they do not need the loop. A whole-sequence test drains a local `VecDeque`: push the keys, run, and the handler pushes the foreground follow-up to the same queue.

## Open

- Whether `run` is generic over a `Queue` trait (a `VecDeque` and a channel both implement it) or has a single-threaded form and a threaded form.
- Where a synchronously pushed follow-up goes relative to pending input (front, for the immediate case) and whether that is fixed by the loop or chosen by the caller.
- How the blocking wait when the queue is empty is expressed without tying the generic loop to a run loop.
