# the event loop

Dispatch turns one event into effects. The effects and the events they cause are decoupled: opening Chrome is one action, and Chrome coming to the foreground is a separate event that arrives later. A handler does not return the follow-up event; it triggers the effect, and the resulting event is delivered like any other input.

## Sync core, async edges

The framework is fully synchronous and never blocks. It dispatches an event to an output and hands that output to the effect handler, both synchronously and in the order events arrived. It knows nothing about threads or async: `next` returns `None` on an empty queue rather than waiting.

All async lives in user land, on both edges, always as "enqueue now (synchronous), handle later (separate)":

- Effects out. The framework hands each output to the handler in order. The handler is thin: it puts the real work on its own queue and returns, and separate workers drain that queue and perform it asynchronously. A worker result that is itself an event (Chrome foregrounded) is pushed back onto the event queue and dispatched like any other input. Ordering is the framework's by default (effects handled in order); the handler may override it, for example a higher-priority path for typing.
- Events in. Something outside the framework (a run loop, OS callbacks, a channel) receives inputs and calls `queue_event` as they arrive. Blocking when idle is that layer's job.

So there are two queues, and the framework touches only one synchronously: the event queue it drains, and the worker queue on the far side of the handler that user land drains asynchronously.

## The queue

Everything that produces events pushes to the event queue, and the framework drains it:

- the keyboard source,
- the foreground watcher,
- a worker, when the async work behind an effect produces an event.

Opening Chrome: the handler schedules "open Chrome" and returns; the foreground watcher, seeing Chrome come up, pushes `Foreground(Chrome)`. Nothing synchronously ties the two.

Single-threaded: the OS delivers input on one thread (`CGEventTap` and the workspace notifications are run-loop sources), so the event queue is a local `VecDeque` on that thread, needing neither `Send` nor `'static`. Multi-threaded: it is a channel (`mpsc`), the sources hold `Sender`s and the loop holds the `Receiver`, and events must be `Send + 'static`. The `'static` is only forced by threads.

## SimpleRunner and the real runner

`bind::SimpleRunner` is the synchronous driver: `queue_event` to push, `next` to process one event (`Option<Option<Output>>`: outer `None` is an empty queue, the inner is dispatch's result), `process_event` to queue-and-process one, plus `len`/`is_empty`. It drains rather than blocks, and an empty queue is resumable (queue more, get output again), so it is not an `Iterator`. It is a convenience, not load-bearing: the framework only needs `dispatch` and `accumulate`, and a consumer can pop-dispatch-hand-off in a handful of lines over its own queue. It is enough for the unit tests and the stdin demo.

The real Mercury runner is the same synchronous dispatch with user land around it: the sources feed the event queue and block when idle (a run loop, or a channel `recv`), the handler schedules work on a worker queue, and worker results that are events feed back in. The framework part never blocks. Later the queue can grow event prioritization; `SimpleRunner` is named to leave room for that. There is no generic `run` in freddie: the loop is small, the queue and the wait-when-empty are user land's, and a generic root drags in an awkward `Resolve<Path<'a> = &'a mut N>` bound. `dispatch` and `accumulate` are the pieces.

## Mercury, concretely

Mercury's two sources are macOS run-loop callbacks: a `CGEventTap` for keys and an `NSWorkspace` observer for the frontmost app. Both fire on the run-loop thread, so the event queue is a plain `VecDeque` behind a `RefCell` (one thread, no `Send`, no `'static`). The one thread boundary is a typing worker: keystroke synthesis is pushed to it over a channel and performed in order, off the hot path, so the tap callback stays fast. Nothing comes back from that worker; the only feedback is the OS foregrounding an app, which the observer reports as a normal event.

```rust
let state = RefCell::new(Mercury::default());
let events = RefCell::new(VecDeque::<MercuryEvent>::new());
let typing = spawn_typing_worker(); // owns the far end of a channel; synthesizes in order

// The key tap is the hot path: quit on escape, else queue the key, drain, and
// tell the OS whether to swallow it. (The tap swallows Ctrl-C, so escape quits.)
on_key(|key_event| {
    if key_event.key == "escape" {
        std::process::exit(0);
    }
    events.borrow_mut().push_back(key(key_event.key));
    if drain(&state, &events, &typing) { Suppress } else { PassThrough }
});

// The frontmost-app observer feeds the same queue.
on_foreground_change(|app| {
    events.borrow_mut().push_back(foreground(app));
    drain(&state, &events, &typing);
});

// The loop: drain the event queue, dispatching each event and handing its effects
// to the handler. Returns whether anything was handled (for the tap's choice).
fn drain(state: &RefCell<Mercury>, events: &RefCell<VecDeque<MercuryEvent>>, typing: &Typing) -> bool {
    let mut handled = false;
    while let Some(event) = events.borrow_mut().pop_front() {
        // dispatch mutates state (layer, foregrounded) and returns the effects
        let Some(output) = state.borrow_mut().handle(&event) else {
            continue;
        };
        handled = true;
        for effect in output {
            match effect {
                // Fire-and-forget: the OS activates the app, and the observer
                // pushes `Foreground` when it actually comes up.
                MercuryEffect::Foreground(app) => activate_app(app),
                // Off the hot path, performed in order by the worker.
                MercuryEffect::Type(k) => typing.send(Synth::Type(k)),
                MercuryEffect::Command(k) => typing.send(Synth::Command(k)),
            }
        }
    }
    handled
}
```

`drain` never blocks; `CFRunLoopRun` does the waiting and fires the callbacks. State is mutated directly inside `handle` (the fast path); the two effects that reach the world are either fire-and-forget with feedback (`Foreground`) or handed to the worker (`Type`, `Command`). The `CGEventTap` / `NSWorkspace` / synthesis calls are the unverified macOS FFI, and belong to figaro.

## Prior art

isograph's language server is the same loop: several sources bridged into one `mpsc`, one event popped and dispatched per iteration, and dispatch is a `ControlFlow` chain (Break on the first matching handler, Continue otherwise), just like freddie's. It has no effect-as-data layer; handlers send LSP messages inline.

barnum is the same deferred-effect shape one step further: its `advance` queues effects rather than performing them, an async scheduler runs handlers off that queue, and completions trampoline back as more effects. Its scheduler is our worker layer. The distinguishing difference is state: a freddie handler mutates state directly during dispatch (it holds `&mut` via the path), while a barnum handler never touches engine state and can only return a value the engine writes back.

## Tests

The per-event tests dispatch one event and assert the output; they do not need a runner. The whole-sequence test drives a `SimpleRunner`: queue a key, drain it (recording effects and queueing an installed app's foreground follow-up), then queue the next key, so it sees "press c, Chrome comes up, press r".

## Open

- Event prioritization: the reason `SimpleRunner` is not just `Runner`. See prioritization.md.
- The concrete shape of the worker queue and how a worker's result re-enters as an event.
- Whether ingestion is single-threaded (run-loop sources, no `'static`) or multi-threaded (a channel, `Send + 'static`).
