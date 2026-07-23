# the model loop is a function you call

Not worth doing, and moved here without being built. The extraction pays off only when a second mercury-shaped app exists to share the loop, and there is none: isograph is a directory watcher, not a model, and a language server or plain server has its own loop, so all of those use `freddie_cli` and run their own thing inside `run_daemon`. With one consumer, `run_standard_loop` is an eight-line function and a two-method trait wrapped around code mercury already has, and mercury inlining the loop is the same size and clearer. Revisit when figaro or another model app lands and would otherwise copy the loop and risk getting the effect-before-event ordering wrong; the design below is what to build then.

A freddie app that runs a model does the same small thing at its core: read an event, hand it to the model, perform the effects the model returned, and stop when one of them says to. Everything around that — grabbing a keyboard, parking the main thread in a run loop, hanging a menu-bar icon, binding a socket, watching a directory — is the app's, and no two apps have the same set.

So the reusable piece is that loop and nothing else. It is a function an app calls, next to `freddie_keyboard::intercept` and `freddie_menu_bar::show`, not a runtime that owns the process and calls back into the app. An app keeps its own `main` and its own setup, builds its channels, and calls the loop with its model.

Nothing macOS-shaped moves. The main-thread run loop, the menu bar, the worker thread, and the sources stay in mercury, wired in mercury. A headless app has none of them: no menu bar, no `CFRunLoop`, and so no worker thread either, because the worker exists only because AppKit demands the main thread and mercury has to run the model somewhere else. A language server or a directory watcher runs this loop straight on the thread it starts on.

## The contract

Two methods. An event in, the effects it produced out, and a way to perform one that can say stop.

```rust
/// A model the standard loop can drive: it turns an event into effects, and performs them.
///
/// The implementing value owns whatever performing an effect needs — an emitter, a socket, a
/// sender for the events an effect produces in turn — so `perform` takes only `&mut self`.
pub trait Model {
    /// What its sources produce and it dispatches.
    type Event;

    /// What dispatch produces and [`perform`](Self::perform) carries out.
    type Effect;

    /// Dispatch one event, returning what it produced. Empty for an event this model ignores.
    fn handle(&mut self, event: &Self::Event) -> Vec<Self::Effect>;

    /// Perform one effect. `Break` ends the loop and returns through [`run_standard_loop`].
    fn perform(&mut self, effect: Self::Effect) -> ControlFlow<()>;
}
```

Two methods on one value rather than two functions handed to the loop. `handle` and `perform` are a matched pair operating on the same running model, and the trait ties them, and the `Event` and `Effect` types, to it: a call site passes one model, not a value plus two function references that could be mismatched or drift apart. The loop is generic over the trait, so the dispatch is still static and inlined.

## The loop

```rust
/// Drive `model` off `events` until an effect breaks or the channel closes.
///
/// One standard way to run a model, not the only one. It performs each event's effects inline, in
/// the order `handle` produced them, so a modifier reaches the OS before the key carrying its
/// flag. A `Break` returns, which drops `model` and everything it owns, so the app's teardown —
/// releasing a grab, closing a socket — is a `Drop` impl rather than a shutdown step.
///
/// The channel closing ends the loop too, but a live source holds a sender, so in a running app
/// the `Break` is the way out.
pub async fn run_standard_loop<M: Model>(model: &mut M, mut events: UnboundedReceiver<M::Event>) {
    while let Some(event) = events.recv().await {
        for effect in model.handle(&event) {
            if model.perform(effect).is_break() {
                return;
            }
        }
    }
}
```

That is the whole thing. It lives in `freddie`, which is already the model core and already depends on tokio's `sync` feature, so it adds no dependency; `ControlFlow` and the receiver are the only imports.

An app whose shape does not fit — a bounded channel, a different runtime, two models multiplexed — writes its own loop against `Model` in a dozen lines. That is why `Model` is the seam and the loop is a convenience over it, and why this one is `run_standard_loop` rather than the only loop there is.

## The dispatch record is the app's

mercury writes one line per dispatch carrying the event, the effects, and the resulting state. The loop cannot: it does not know the model is printable and must not require it. So the record stays where the state is, inside `handle`, and the loop stays free of tracing entirely.

```rust
    fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> {
        let effects = self.state.handle(event).unwrap_or_default();
        info!(event = ?event, effects = ?effects, state = ?self.state, "dispatch");
        effects
    }
```

## Why the channel is concrete

An `impl` bound over the receiver was considered, so the loop would take any channel rather than tokio's. Against: the function's value is being a handful of obvious lines, and tokio exposes no receiver trait, so abstracting it means inventing one or pulling in `futures::Stream` — a bound and a dependency hung on an eight-line helper. An app that needs a different channel writes its own loop against `Model`, which the two-method trait makes trivial, so the generality has a home already and the standard loop stays concrete.

## Stopping is a source

A signal is an event source like any other. mercury installs a SIGTERM handler that sends its quit event into the event channel; it stays in mercury's `serve`, beside the socket and the keyboard grab. The loop knows nothing about signals: a quit arrives as an ordinary event, the model turns it into the effect that breaks, and the loop returns.

```rust
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: quitting");
                    let _ = event_tx.send(quit_event());
                }
            });
        }
        Err(e) => warn!(error = %e, "no SIGTERM handler; a terminated mercury will not quit cleanly"),
    }
```

Nothing in `freddie` routes signals, because which event a signal means, and what has to be undone before the process may go, is the app's, and a headless app answers differently.

## What mercury becomes

`crates/mercury/src/daemon.rs` keeps the setup in `serve` and the process arrangement in `run`, and loses the two loops. The event loop, the effect loop, `dispatch_event`, and the effect channel all go; `perform_effect` becomes a method.

The performance context the effect loop carried as arguments — the emitter, the title channel, the window sink, the overlay, the event sender for follow-up events — becomes the fields of the value the loop drives:

```rust
/// Mercury, running: the model and everything performing an effect needs alive.
pub(crate) struct MercuryDaemon {
    state: Mercury,
    emitter: Emitter,
    event_tx: UnboundedSender<MercuryEvent>,
    title_tx: std::sync::mpsc::Sender<&'static str>,
    windows: Option<WindowSink>,
    overlay: OverlaySink,
}

impl Model for MercuryDaemon {
    type Event = MercuryEvent;
    type Effect = MercuryEffect;

    fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> { .. } // the record, above

    fn perform(&mut self, effect: MercuryEffect) -> ControlFlow<()> {
        // today's `perform_effect` body, its `emitter`/`event_tx`/`title_tx`/`windows`/`overlay`
        // parameters now `self.` fields.
    }
}
```

`serve` keeps binding the socket, grabbing the keyboard, installing the SIGTERM source, and sending the boot layer's name, then builds the daemon and hands it to the loop. Before, its tail:

```rust
    let mercury = Mercury::new(boot.front_app, boot.windows);
    let _ = title_tx.send(mercury.layer().name());

    tokio::select! {
        () = run_event_loop(mercury, event_rx, effect_tx) => {}
        () = run_effect_loop(effect_rx, emitter, event_tx, title_tx, boot.window_sink, boot.overlay) => {}
    }
    drop(interceptor); // hold the grab until here
```

After, with the `let (effect_tx, effect_rx) = ..` at the top of `serve` gone too:

```rust
    let mut daemon = MercuryDaemon {
        state: Mercury::new(boot.front_app, boot.windows),
        emitter,
        event_tx,
        title_tx,
        windows: boot.window_sink,
        overlay: boot.overlay,
    };
    let _ = daemon.title_tx.send(daemon.state.layer().name());

    freddie::run_standard_loop(&mut daemon, event_rx).await;
    drop(interceptor); // hold the grab until here
```

`daemon::run`, which parks the main thread, shows the menu bar, and spawns the worker that block_on's `serve`, does not change: that is the macOS menu-bar arrangement, and it is mercury's.

## The changes, in order

1. **`Model` and `run_standard_loop` in `freddie`.** The trait and the eight-line loop, no new dependency.
2. **mercury implements it.** `MercuryDaemon` with the effect context as fields, `perform_effect` becoming `perform` and the dispatch record moving into `handle`. `serve` builds the daemon and calls `run_standard_loop`; `run_event_loop`, `run_effect_loop`, `dispatch_event`, and the effect channel are deleted. No behaviour changes: the same events dispatch to the same effects in the same order, and the log keeps its one line per dispatch.
