# the event loop

Dispatch turns one event into effects. A runner feeds it events and performs the effects. The effects and the events they cause are decoupled: opening Chrome is one action, and Chrome coming to the foreground is a separate event that arrives later. A handler does not return the follow-up event; it triggers the effect, and the resulting event is delivered like any other input.

## Sync core, async edges

The framework is synchronous up to the handoff. Dispatch is one event to an output, and the runner hands each output to the handler synchronously, in the order the events were dispatched. Nothing in the framework knows about threads or async.

Performing the output is the handler's business. The framework handles effects in order, so typed characters do not come out of order by default; a handler is free to impose its own policy on top, for example a higher-priority path for typing. Handling can also be async: a quick handler schedules the real work (later, on a worker pool) and returns.

The async also lives at ingestion. Several sources (the keyboard tap, the foreground watcher) produce events concurrently, write into one queue without blocking, and the runner reads from it separately. Getting an event, queueing it, and reading it are decoupled, and as many can queue as arrive.

So the runner's job is small: read the next event, dispatch it, hand the output to the handler in order. It never waits on the effect being performed and never knows an effect is async.

## The queue

Everything that produces events pushes to one queue, and the runner drains it:

- the keyboard source,
- the foreground watcher,
- the effect handler, for an event it knows synchronously.

Opening Chrome: the handler tells the OS to open Chrome and returns. The foreground watcher, seeing Chrome come up, pushes `Foreground(Chrome)`. Nothing synchronously ties the two.

Single-threaded: the OS delivers input on one thread (`CGEventTap` and the workspace notifications are run-loop sources), so the queue is a local `VecDeque` on that thread, needing neither `Send` nor `'static`. Multi-threaded: the queue is a channel (`mpsc`), the sources hold `Sender`s, the runner holds the `Receiver`, and events must be `Send + 'static`. The `'static` is only forced by threads.

## SimpleRunner and the real runner

`bind::SimpleRunner` is the synchronous driver: `queue_event` to push, `next` to process one event (`Option<Option<Output>>`: outer `None` is an empty queue, the inner is dispatch's result), `process_event` to queue-and-process one, plus `len`/`is_empty`. It drains rather than blocks, and an empty queue is resumable (queue more, get output again), so it is not an `Iterator`. It is enough for the unit tests and the stdin demo.

The real Mercury runner is the same synchronous core with async ingestion in front: the sources feed a queue, the runner reads it and blocks when idle. Later it grows event prioritization. `SimpleRunner` is named to leave room for it. There is no generic `run` in freddie: the loop is small, its queue and its wait-when-empty differ per consumer, and a generic root drags in an awkward `Resolve<Path<'a> = &'a mut N>` bound. `dispatch` and `accumulate` are the pieces.

## Tests

The per-event tests dispatch one event and assert the output; they do not need a runner. The whole-sequence test drives a `SimpleRunner`: queue a key, drain it (recording effects and queueing an installed app's foreground follow-up), then queue the next key, so it sees "press c, Chrome comes up, press r".

## Open

- Event prioritization: the reason `SimpleRunner` is not just `Runner`.
- How an effect handler hands long work to a worker pool while staying synchronous to the framework.
- Whether the real runner's ingestion is single-threaded (run-loop sources, no `'static`) or multi-threaded (a channel, `Send + 'static`).
