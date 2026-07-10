//! Mercury's runnable v1: grab the keyboard and drive the model.
//!
//! `freddie_keyboard::intercept` swallows every key and hands each to the model.
//! Two loops over two channels: the event loop dispatches (mutating state,
//! producing effects); the effect loop performs them, re-emitting keys through the
//! `Emitter` and foregrounding apps through `freddie_app_nav`. A
//! `freddie_app_nav` watcher runs the app-navigation source: it reports the
//! frontmost app as a `Foreground` event, so foregrounding an app (the effect) and
//! observing it come up (the event) are decoupled the way the model expects.
//!
//! Every key goes through the model, so `escape` is handled there (it goes home)
//! and `q` from home quits. A 30-second timer is the backstop out of the hijack
//! (hard exit 5s after that).
//!
//! Quitting is a `Kill` effect, which ends the effect loop rather than exiting the
//! process, so the way out runs destructors: the keyboard is released and main's
//! run loop is stopped. The hard exit is only for a wedged process.
//!
//! # Threads
//!
//! Three, each asleep in its own loop, joined only by a channel.
//!
//! The main thread runs the platform run loop and nothing else, so that `AppKit`
//! can deliver callbacks there. It is a doorman: a callback sends into the event
//! channel and returns. Main-thread callbacks are serialized, so a slow one would
//! stall every other source.
//!
//! The tap thread, spawned inside `freddie_keyboard::intercept`, runs its own run
//! loop for the `CGEventTap`. It has always been off main, which is why the
//! keyboard works whatever main is doing.
//!
//! The worker thread runs the tokio runtime, owns the state and the `!Send`
//! `Emitter`, and runs both the event and effect loops. It is the only place
//! state is mutated, so there is no shared mutable state and no `Mutex`.
//!
//! On macOS this needs Accessibility (and Input Monitoring). `cargo run -p mercury`

use std::ops::ControlFlow;
use std::time::Duration;

use freddie_keyboard::Emitter;
use mercury::{App, Mercury, MercuryEffect, MercuryEvent, foreground};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, error, info, warn};

mod logging;

/// Give the main thread to the run loop, and run mercury on a worker thread.
///
/// `AppKit` delivers its callbacks only while the main thread is inside a run
/// loop, so main sits in one and mercury runs in [`run`] elsewhere. See
/// `refactors/past/main-thread.md`.
///
/// Dropping the worker's `Stopper` stops main's loop, so a normal return, a
/// failed keyboard grab, and a panic all exit. Declaration order below matters:
/// the runtime drops before the `Stopper`.
fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    let worker = std::thread::Builder::new()
        .name("mercury-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(run());
        })
        .expect("spawning the runtime thread");

    main_loop.run(); // services AppKit sources until the worker drops the stopper
    let _ = worker.join();
}

/// Everything mercury does, on the worker thread.
///
/// `intercept` has to be called from here rather than from `main`, because it
/// returns the `Emitter`, the `Emitter` is `!Send` (it holds an `Rc`), and the
/// effect loop uses it. It has to be born on the thread it will live on.
///
/// Which is exactly why this future is `!Send`, and why that is fine: it is
/// `block_on`ed by the worker's current-thread runtime and never crosses a
/// thread.
#[allow(clippy::future_not_send)]
async fn run() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // Grab the keyboard: swallow every key and forward it to the model, which
    // decides what to emit (the effect loop performs it).
    let grabbed = freddie_keyboard::intercept({
        let event_tx = event_tx.clone();
        move |ev| {
            // Forward every key, down and up, with its real press. Dropping the up
            // leaves a modifier stuck down in the emitted stream: after ctrl-a, a
            // swallowed ctrl-up means the next key still carries ctrl (p arrives as
            // ctrl-p). We always swallow here; the model dispatches and the effect
            // loop re-emits whatever passes through.
            let _ = event_tx.send(MercuryEvent::Key(ev));
            None
        }
    });
    let (interceptor, emitter) = match grabbed {
        Ok(pair) => pair,
        Err(e) => {
            // Usually Accessibility is not granted. Say so on the terminal too: the
            // user is looking there, not at the log file.
            eprintln!("keyboard: {e}");
            error!(error = %e, "could not intercept the keyboard");
            return;
        }
    };

    // The app-navigation source. `watch` reports changes, not the app that is
    // already frontmost, so seed that one by hand.
    if let Some(bundle_id) = freddie_app_nav::frontmost() {
        let _ = event_tx.send(foreground(App::from_bundle_id(&bundle_id)));
    }
    // The block runs on the main thread, where callbacks are serialized, so it does
    // one send and returns. The work happens back on this thread.
    let _watcher = freddie_app_nav::watch({
        let event_tx = event_tx.clone();
        move |bundle_id| {
            let _ = event_tx.send(foreground(App::from_bundle_id(bundle_id)));
        }
    });

    spawn_killswitch(effect_tx.clone());

    println!("mercury: hijacking the keyboard; escape then q quits (30s backstop)");
    info!("hijacking the keyboard; escape then q quits (30s backstop)");
    // `select!` rather than `join!`: the effect loop ends on `Kill`, and the event
    // loop never does, because the tap thread holds a sender for as long as the
    // grab is alive.
    tokio::select! {
        () = run_event_loop(Mercury::default(), event_rx, effect_tx) => {}
        () = run_effect_loop(effect_rx, emitter) => {}
    }
    drop(interceptor); // hold the grab until here
}

/// Dev killswitch: a `Kill` effect after 30s, then a hard exit 5s later if the
/// clean path never got there.
///
/// The hard exit runs no destructors, which is the point: it is the backstop for a
/// wedged process that is still swallowing the keyboard. Reaching it means the
/// clean exit failed.
fn spawn_killswitch(effect_tx: UnboundedSender<MercuryEffect>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let _ = effect_tx.send(MercuryEffect::Kill);
        tokio::time::sleep(Duration::from_secs(5)).await;
        std::process::exit(1);
    });
}

/// The event loop: read the event channel and dispatch each event.
async fn run_event_loop(
    mut state: Mercury,
    mut event_rx: UnboundedReceiver<MercuryEvent>,
    effect_tx: UnboundedSender<MercuryEffect>,
) {
    info!(state = ?state, "initial state");
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &effect_tx);
    }
}

/// Dispatch one event through freddie and enqueue whatever effects it produced.
///
/// One record per dispatch, carrying the event, the effects it produced, and the
/// state it left behind, so a single line tells the whole story of one event.
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let effects = state.handle(event).unwrap_or_default();
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}

/// The effect loop: read the effect channel and perform each effect, until one of
/// them says to stop.
///
/// The `Emitter` is `!Send` by design, so this future is `!Send`; it runs on the
/// worker thread that created the `Emitter` and never crosses a thread.
#[allow(clippy::future_not_send)]
async fn run_effect_loop(mut effect_rx: UnboundedReceiver<MercuryEffect>, emitter: Emitter) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(&effect, &emitter).is_break() {
            break;
        }
    }
}

/// Perform one effect: emit keys, foreground an app, or stop. The effect itself is
/// already on the dispatch record; these are the performance details.
///
/// `Kill` breaks rather than exiting the process, so the way out runs destructors:
/// the `Interceptor` releases the keyboard, the `Stopper` stops main's run loop,
/// and anything registered with the OS gets to deregister.
fn perform_effect(effect: &MercuryEffect, emitter: &Emitter) -> ControlFlow<()> {
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(*app),
        MercuryEffect::Emit(ke) => match emitter.emit(ke.key, ke.press) {
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
            Err(e) => warn!(key = ?ke.key, press = ?ke.press, error = %e, "emit failed"),
        },
        MercuryEffect::Kill => {
            info!("kill: exiting");
            return ControlFlow::Break(());
        }
    }
    ControlFlow::Continue(())
}

/// Foreground an app for real, fire-and-forget on its own thread so the effect
/// loop never blocks on `open`. The watcher reports the app that actually comes
/// up, so nothing here waits on the result (see `app-foregrounding.md`).
fn foreground_app(app: App) {
    let Some(bundle_id) = app.bundle_id() else {
        warn!(app = ?app, "no bundle id; not foregrounding");
        return;
    };
    std::thread::spawn(move || match freddie_app_nav::foreground(bundle_id) {
        Ok(()) => debug!(app = bundle_id, "foregrounded"),
        Err(e) => warn!(app = bundle_id, error = %e, "foreground failed"),
    });
}
