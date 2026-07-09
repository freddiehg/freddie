# dispatch and effects

Every event is dispatched the moment it arrives, synchronously, and dispatch returns a `Vec` of effects. For a key that happens inside the tap callback, so the effects are ready before the callback returns. That is what lets the key output go back down the tap chain as the callback's return value instead of being re-posted, which is what keeps it correct and loop-free (cgevent-vs-hid.md).

## Two kinds of effect

The effects a handler returns split by where they go:

- Key output, the remap result. Applied synchronously. In the tap callback it is the return value: pass the event, replace it, or drop it, with any extra keys posted through the tap proxy right there. No channel, no re-post.
- Everything else: foregrounding an app, launching, anything that touches I/O. Handed to a thread pool and forgotten. macOS disables a tap whose callback runs long, so offloading these is not a nicety, it is what keeps the tap alive.

The result of a slow effect comes back as its own event. Activating an app does not update state directly; the foreground watcher reports the app that actually came up, and that event dispatches like any other.

## Effects and their events are decoupled

A handler does not return the follow-up event. Opening Chrome is one effect; Chrome coming to the foreground is a separate event that arrives later from the watcher. The handler triggers the effect and moves on, and nothing synchronously ties the two. That is why a slow effect can go to the pool with no one waiting on it.

## State

Keys dispatch on the tap thread. The foreground watcher dispatches on its own thread. Both mutate the one state tree, so access is serialized: a `Mutex` with short critical sections, or every event marshaled onto one thread's run loop. Dispatch is microseconds and keyboard rates are low, so the `Mutex` is fine, and the lock is never held across a slow effect since those are already offloaded.

## Sources

- The keyboard: the tap callback, dispatching synchronously and returning the key output.
- The foreground watcher: its own thread, feeding `Foreground` events.
- Pool workers: performing slow effects and feeding their results back as events.

There is no drain-a-channel event loop and no async runtime on the key path. Dispatch runs where the event arrives. A thread pool, or tokio's blocking pool if one is already around, handles the slow effects and nothing else.

## SimpleRunner, for tests

`bind::SimpleRunner` is the synchronous test driver: `queue_event` to push, `next` to dispatch one (`Option<Option<Output>>`, outer `None` for an empty queue), `process_event` to queue-and-process, plus `len` and `is_empty`. It drains rather than blocks and an empty queue is resumable, so it is not an `Iterator`. The per-event tests dispatch one event and assert the effects with no runner; the sequence test drives a `SimpleRunner`, queueing a key, draining it while recording effects and queueing a foreground follow-up, then the next key, so it sees "press c, Chrome comes up, press r".

## Killswitch while developing

The real build swallows the keyboard, so a dead-man switch stays in until the tap is trusted. A timer fires a `Kill` effect at 5 seconds, a clean exit that drops the tap and restores the keyboard; a second timer calls `_exit` at 10 seconds as the backstop for a wedged process. `SIGHUP` stays fatal, so closing the launching terminal kills it. The keyboard is back within ten seconds without a reboot, which is what makes iterating on the tap safe.

## Prior art

isograph's language server bridges several sources into one `mpsc` and dispatches one popped event per `select!` iteration, with dispatch a `ControlFlow` chain (Break on the first matching handler). freddie's key path is tighter: it dispatches in the tap callback and returns, no channel in between, because the key output has to be synchronous. barnum defers every effect to an async scheduler; freddie defers only the slow ones and applies key output synchronously. The shared shape is effects as data and follow-ups as events; the difference is that a freddie handler mutates state during dispatch through the path it holds.

## Mercury, concretely

The tap thread runs the `CGEventTap`. Its callback locks state, dispatches the key, and splits the effects: key output becomes the return value (plus any extra posts through the proxy), everything else goes to the pool. The foreground watcher runs on its own thread and feeds `Foreground` events, which dispatch under the same lock. A small thread pool performs the slow effects.

```rust
// keyboard: the tap callback, synchronous
let effects = {
    let mut state = state.lock();
    state.handle(&KeyEvent { key, press })   // dispatch mutates state, returns Vec<Effect>
};
perform_slow(&effects, &pool);              // foreground, launch, I/O -> pool, fire-and-forget
return key_output(&effects);                // the key effects -> Keep / Replace / Drop, into the chain
```

No effect blocks the callback: the slow ones are offloaded, the key ones are a return value. The macOS calls (the tap, the watcher, key synthesis, app activation) are the unverified FFI.

## Open

- Event prioritization: prioritization.md, the reason `SimpleRunner` is not just `Runner`.
- One-to-many key output: returning the first key and posting the rest through the proxy has an ordering question; v1 is mostly one-to-one.
- Serialize state with a `Mutex` or a single run-loop thread; the `Mutex` is the v1 default.
