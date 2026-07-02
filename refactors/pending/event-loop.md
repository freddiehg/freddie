# the event loop

Dispatch turns one event into effects. The effects and the events they cause are decoupled: opening Chrome is one action, and Chrome coming to the foreground is a separate event that arrives later. A handler does not return the follow-up event; it triggers the effect, and the resulting event is delivered like any other input.

## Sync core, async edges

The framework is fully synchronous and never blocks. It dispatches an event to an output and hands that output to the effect handler, both synchronously and in the order events arrived. It knows nothing about threads or async: `next` returns `None` on an empty queue rather than waiting.

All async lives in user land, on both edges, always as "enqueue now (synchronous), handle later (separate)":

- Effects out. Dispatch returns the effects, and the event loop hands each one to an effect channel and moves on. Enqueuing never blocks, so a slow effect never back-pressures event handling. A separate effect loop reads that channel in order and performs each effect; v1 performs them synchronously, and later a slow effect can be offloaded to a pool there (keyboard synthesis staying synchronous) without the event loop noticing. A performed effect whose result is itself an event (Chrome foregrounded) reaches the event channel like any other input.
- Events in. Something outside the framework (a run loop, OS callbacks, a channel) receives inputs and hands them to dispatch as they arrive. Blocking when idle is that layer's job.

So there are two channels, and the framework's dispatch touches only the first: the event channel the event loop drains, and the effect channel the effect loop drains.

## The queue

Everything that produces events pushes to the event queue, and the loop drains it:

- the keyboard source,
- the foreground watcher,
- a worker, when the async work behind an effect produces an event.

Opening Chrome: the handler schedules "open Chrome" and returns; the foreground watcher, seeing Chrome come up, pushes `Foreground(Chrome)`. Nothing synchronously ties the two.

Single-threaded: the OS delivers input on one thread and the queue is a local `VecDeque`, no `Send`, no `'static`. Multi-threaded: the queue is a channel (`mpsc`), the sources hold `Sender`s and the loop holds the `Receiver`, and events must be `Send + 'static`. The `'static` is only forced by threads. Mercury takes the channel route (below), so events are `Send + 'static`.

## SimpleRunner and the real runner

`bind::SimpleRunner` is the synchronous driver: `queue_event` to push, `next` to process one event (`Option<Option<Output>>`: outer `None` is an empty queue, the inner is dispatch's result), `process_event` to queue-and-process one, plus `len`/`is_empty`. It drains rather than blocks, and an empty queue is resumable (queue more, get output again), so it is not an `Iterator`. It is a convenience, not load-bearing: the framework only needs `dispatch` and `accumulate`, and a consumer can pop-dispatch-hand-off in a handful of lines over its own queue. It is enough for the unit tests and the stdin demo.

The real Mercury runner is the same synchronous dispatch with user land around it: the sources feed a channel, the loop `select!`s over it and its timers, the handler schedules work on a worker queue, and worker results that are events feed back in. The framework part never blocks. Later the queue can grow event prioritization; `SimpleRunner` is named to leave room for that. There is no generic `run` in freddie: the loop is small, the queue and the wait-when-empty are user land's, and a generic root drags in an awkward `Resolve<Path<'a> = &'a mut N>` bound. `dispatch` and `accumulate` are the pieces.

## Mercury, concretely

Mercury runs two loops joined by two tokio channels (tokio from day one). Each macOS source runs on a thread that owns a `CFRunLoop` and forwards into the event channel: a `CGEventTap` for keys, an `NSWorkspace` observer for the frontmost app. The event loop reads that channel, dispatches (which mutates state synchronously and returns effects), and hands each effect to the effect channel; a separate effect loop reads the effect channel and performs them. `dispatch` stays synchronous inside the event loop, and this is the same shape as isograph's language server.

```rust
#[tokio::main]
async fn main() {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<MercuryEvent>();
    let (effect_tx, mut effect_rx) = tokio::sync::mpsc::unbounded_channel::<MercuryEffect>();

    spawn_key_tap(event_tx.clone());            // its own CFRunLoop thread; forwards key events
    spawn_foreground_watcher(event_tx.clone()); // forwards Foreground events

    // Effect loop: read the effect channel and perform, in order. Sync for now;
    // a slow effect could later be spawned onto a pool here instead.
    tokio::spawn(async move {
        while let Some(effect) = effect_rx.recv().await {
            match effect {
                MercuryEffect::Foreground(app) => activate_app(app), // fire-and-forget; watcher reports back
                MercuryEffect::Type(k) => synthesize_type(k),        // fast CGEventPost
                MercuryEffect::Command(k) => synthesize_command(k),
            }
        }
    });

    // Event loop: read the event channel, dispatch, and enqueue the effects.
    let mut state = Mercury::default();
    let graceful = tokio::time::sleep(Duration::from_secs(5)); // dev killswitch
    let hard = tokio::time::sleep(Duration::from_secs(10));    // dev killswitch
    tokio::pin!(graceful, hard);
    loop {
        let event = tokio::select! {
            Some(event) = event_rx.recv() => event,
            () = &mut graceful => MercuryEvent::Kill, // 5s: graceful
            () = &mut hard => libc::_exit(1),         // 10s: hard
        };
        if matches!(event, MercuryEvent::Kill) {
            break; // clean exit: drop the tap, keyboard restored
        }
        // dispatch mutates state synchronously and returns the effects
        let Some(output) = state.handle(&event) else { continue };
        for effect in output {
            let _ = effect_tx.send(effect); // the sync handler just enqueues
        }
    }
}
```

`select!` is the wait; `handle` never blocks. Dispatch mutates state directly (the fast path) and returns the effects; the event loop only enqueues them (unbounded, so the send never blocks), so it never waits on an effect. The effect loop performs the two outward things: send keyboard events (`Type`, `Command`) and foreground apps (`Foreground`, fire-and-forget; the observer reports the app coming up as a normal event). v1 performs them synchronously and in order (one consumer, FIFO); later a slow effect can be offloaded to a pool inside the effect loop, keyboard synthesis staying synchronous, without the event loop noticing. Events and effects cross threads, so both are `Send + 'static`. The `CGEventTap` / `NSWorkspace` / synthesis / activation calls are the unverified macOS FFI and belong to figaro.

### Killswitches while developing

Swallowing every key means a bug can lock the machine out, so while the tap is being built there are three dead-man switches, kept in until it is trusted:

- A `Kill` event (a new `MercuryEvent` variant). It arrives through the channel like any input, and the loop turns it into a clean exit that drops the tap and restores the keyboard. A killswitch key (Ctrl-C, which the tap would otherwise swallow) sends it too.
- The 5-second `select!` arm sends `Kill` (graceful); the 10-second arm calls `libc::_exit` outright (hard), in case the graceful path is wedged. Either way the keyboard is back within ten seconds, no reboot.
- `SIGHUP` stays fatal, so closing the terminal that launched it kills it. Do not install a handler that swallows `SIGHUP`.

The worst case self-recovers on a timer, which is what makes it safe to iterate on the tap at all.

## Prior art

isograph's language server is the same loop: several sources bridged into one `mpsc`, one event popped and dispatched per `select!` iteration, and dispatch is a `ControlFlow` chain (Break on the first matching handler, Continue otherwise), just like freddie's. It has no effect-as-data layer; handlers send LSP messages inline.

barnum is the same deferred-effect shape one step further: its `advance` queues effects rather than performing them, an async scheduler runs handlers off that queue, and completions trampoline back as more effects. Its scheduler is our worker layer. The distinguishing difference is state: a freddie handler mutates state directly during dispatch (it holds `&mut` via the path), while a barnum handler never touches engine state and can only return a value the engine writes back.

## Tests

The per-event tests dispatch one event and assert the output; they do not need a runner. The whole-sequence test drives a `SimpleRunner`: queue a key, drain it (recording effects and queueing an installed app's foreground follow-up), then queue the next key, so it sees "press c, Chrome comes up, press r".

## Open

- Event prioritization: the reason `SimpleRunner` is not just `Runner`. See prioritization.md.
- Post-v1: a pool for slow effects (keyboard synthesis stays synchronous, only slow effects offload), purely to avoid back pressure on event handling.
- macOS FFI feasibility (figaro): keyboard tap + swallow (Accessibility and Input Monitoring, plus the self-lockout footgun), synthesizing keys (Accessibility, and tagging events so the tap ignores its own), foreground watching (easy, no permission), activating apps (easy).
