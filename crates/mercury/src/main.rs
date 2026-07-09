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
//! The `Emitter` is `!Send`, so the effect loop runs on this task via `join!`
//! rather than a spawned task.
//!
//! On macOS this needs Accessibility (and Input Monitoring). `cargo run -p mercury`

use std::time::Duration;

use freddie_keyboard::Emitter;
use mercury::{App, Mercury, MercuryEffect, MercuryEvent, foreground};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // Grab the keyboard: swallow every key and forward key-downs to the model,
    // which decides what to emit (the effect loop performs it).
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
            eprintln!("keyboard: {e}"); // usually Accessibility is not granted
            return;
        }
    };

    // The app-navigation source: report the frontmost app as a foreground event.
    let _watcher = freddie_app_nav::watch(freddie_app_nav::DEFAULT_POLL_INTERVAL, {
        let event_tx = event_tx.clone();
        move |name| {
            let _ = event_tx.send(foreground(App::from_name(name)));
        }
    });

    spawn_killswitch(effect_tx.clone());

    println!("mercury: hijacking the keyboard; escape then q quits (30s backstop)");
    tokio::join!(
        run_event_loop(Mercury::default(), event_rx, effect_tx),
        run_effect_loop(effect_rx, emitter),
    );
    drop(interceptor); // hold the grab until here
}

/// Dev killswitch: a `Kill` effect after 30s (the effect loop exits on it), then a
/// hard exit 5s later if that never happened.
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
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &effect_tx);
    }
}

/// Dispatch one event through freddie, log the event, its effects, and the
/// resulting state, then enqueue the effects.
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let effects = state.handle(event).unwrap_or_default();
    eprintln!("event {event:?} -> {effects:?}\n  state {state:?}");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}

/// The effect loop: read the effect channel and perform each effect.
///
/// The `Emitter` is `!Send` by design, so this future is `!Send`; `main` runs it
/// on the current thread via `join!` and it never crosses a thread.
#[allow(clippy::future_not_send)]
async fn run_effect_loop(mut effect_rx: UnboundedReceiver<MercuryEffect>, emitter: Emitter) {
    while let Some(effect) = effect_rx.recv().await {
        perform_effect(&effect, &emitter);
    }
}

/// Perform one effect: emit keys, foreground an app, or exit.
fn perform_effect(effect: &MercuryEffect, emitter: &Emitter) {
    eprintln!("effect {effect:?}");
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(*app),
        MercuryEffect::Emit(ke) => {
            if let Err(e) = emitter.emit(ke.key, ke.press) {
                eprintln!("emit: {e}");
            }
        }
        MercuryEffect::Kill => std::process::exit(0),
    }
}

/// Foreground an app for real, fire-and-forget on its own thread so the effect
/// loop never blocks on `open`. The watcher reports the app that actually comes
/// up, so nothing here waits on the result (see `app-foregrounding.md`).
fn foreground_app(app: App) {
    let Some(name) = app.launch_name() else {
        eprintln!("foreground: {app:?} has no launch name");
        return;
    };
    std::thread::spawn(move || {
        if let Err(e) = freddie_app_nav::foreground(name) {
            eprintln!("foreground {name}: {e}");
        }
    });
}
