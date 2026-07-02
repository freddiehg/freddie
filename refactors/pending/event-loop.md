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

The real Mercury runner is the same synchronous dispatch with user land around it: sources feed the event channel, the event loop reads it and dispatches, the effect loop performs the effects, and an effect that is itself an event (a foregrounded app) feeds back in. The framework part never blocks. Later the effect loop can offload a slow effect to a pool and the queue can grow event prioritization; `SimpleRunner` is named to leave room for that. There is no generic `run` in freddie: the loops are small, the channels and the wait-when-empty are user land's, and a generic root drags in an awkward `Resolve<Path<'a> = &'a mut N>` bound. `dispatch` and `accumulate` are the pieces.

## Mercury, concretely

Mercury's v1 runner (`crates/mercury/src/main.rs`) is three loops over two tokio channels, the shape the real build keeps — it swaps the stdin source for a `CGEventTap` thread and printing for OS key synthesis and app activation. A source forwards events into the event channel; the event loop reads it and dispatches each event (mutating state synchronously, returning effects) into the effect channel; the effect loop reads the effect channel and performs each effect. `dispatch` stays synchronous inside the event loop, the same shape as isograph's language server.

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    spawn_stdin_source(event_tx.clone());  // 1. source -> event channel (real build: a CGEventTap thread)
    spawn_killswitch(effect_tx.clone());   // dev safety, below
    tokio::spawn(run_effect_loop(effect_rx, event_tx.clone()));
    run_event_loop(Mercury::default(), event_rx, effect_tx).await;
}

// 2. event channel -> dispatch -> effect channel
while let Some(event) = event_rx.recv().await {
    if let Some(effects) = state.handle(&event) {   // dispatch mutates state, returns effects
        for effect in effects {
            let _ = effect_tx.send(effect);          // enqueue; unbounded, never blocks
        }
    }
}

// 3. effect channel -> perform (v1 prints; the real build synthesizes / activates)
match effect {
    MercuryEffect::Foreground(app) => activate_app(*app), // fire-and-forget; watcher reports back
    MercuryEffect::Type(k) => synthesize_type(k),
    MercuryEffect::Command(k) => synthesize_command(k),
    MercuryEffect::Kill => std::process::exit(0),         // the kill, in the effect handler
}

// killing is an effect (5s, graceful) or a direct hard exit (10s)
fn spawn_killswitch(effect_tx: UnboundedSender<MercuryEffect>) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        let _ = effect_tx.send(MercuryEffect::Kill);  // performed by the effect loop
        sleep(Duration::from_secs(5)).await;
        std::process::exit(1);                         // hard backstop
    });
}
```

Dispatch mutates state directly (the fast path) and returns the effects; the event loop only enqueues them (unbounded, so the send never blocks), so it never waits on an effect. The effect loop performs the two outward things: send keyboard events (`Type`, `Command`) and foreground apps (`Foreground`, fire-and-forget; the watcher reports the app coming up as a normal event, which is why v1 feeds a follow-up back itself). v1 performs them synchronously and in order (one consumer, FIFO); later a slow effect can be offloaded to a pool inside the effect loop, keyboard synthesis staying synchronous, without the event loop noticing. Events and effects cross threads, so both are `Send + 'static`. The `CGEventTap` / `NSWorkspace` / synthesis / activation calls are the unverified macOS FFI and belong to figaro.

### Killswitches while developing

v1 uses stdin and does not swallow the keyboard, so there is no lockout risk and Ctrl-C already works. The timers are the dead-man switch for the real build, which does swallow the keyboard; they stay in until the tap is trusted:

- Killing is an effect. The 5-second timer puts `MercuryEffect::Kill` on the effect channel, and the effect loop performs it as a clean exit (in the real build, dropping the tap and restoring the keyboard). A killswitch key (later) reaches the same effect.
- The 10-second timer calls `_exit` directly (hard), the backstop for when the effect loop itself is wedged. Either way the keyboard is back within ten seconds, no reboot.
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
