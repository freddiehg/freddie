//! Mercury's runnable v1: grab the keyboard and drive the model.
//!
//! `freddie_keyboard::intercept` swallows every key and hands each to the model.
//! Two loops over two channels: the event loop dispatches (mutating state,
//! producing effects); the effect loop performs them, re-emitting keys through the
//! `Emitter` and reporting a `Foreground` back as an event (standing in for the OS
//! watcher). The interceptor callback exits on `escape` (the way out of a full
//! hijack); a 5-second timer is the backstop (10-second hard exit).
//!
//! The `Emitter` is `!Send`, so the effect loop runs on this task via `join!`
//! rather than a spawned task.
//!
//! On macOS this needs Accessibility (and Input Monitoring). `cargo run -p mercury`

use std::time::Duration;

use freddie_keyboard::Emitter;
use mercury::{
    AppLayer, Key, Layer, Mercury, MercuryEffect, MercuryEvent, PressType, foreground, key,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // Grab the keyboard: swallow every key, forward key-downs to the model, and
    // exit on escape. escape does not depend on the model or the channel.
    let grabbed = freddie_keyboard::intercept({
        let event_tx = event_tx.clone();
        move |ev| {
            // TODO: swap eprintln for a real logger (tracing) once we have one.
            eprintln!("event: {:?} {:?}", ev.key, ev.press);
            if ev.key == Key::Escape {
                std::process::exit(0);
            }
            if ev.press == PressType::Down {
                let _ = event_tx.send(key(ev.key));
            }
            None // swallow; the model dispatches and the effect loop re-emits
        }
    });
    let (interceptor, emitter) = match grabbed {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("keyboard: {e}"); // usually Accessibility is not granted
            return;
        }
    };

    spawn_killswitch(effect_tx.clone());

    println!("mercury: hijacking the keyboard; escape quits (5s backstop)");
    tokio::join!(
        run_event_loop(Mercury::default(), event_rx, effect_tx),
        run_effect_loop(effect_rx, event_tx, emitter),
    );
    drop(interceptor); // hold the grab until here
}

/// Dev killswitch: a `Kill` effect after 5s (the effect loop exits on it), then a
/// hard exit after 10s if that never happened.
fn spawn_killswitch(effect_tx: UnboundedSender<MercuryEffect>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
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
    log_state(&state);
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &effect_tx);
    }
}

/// Dispatch one event through freddie and enqueue whatever effects it produced.
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let Some(effects) = state.handle(event) else {
        return; // unbound: no state change
    };
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
    log_state(state);
}

/// Print the current layer and foregrounded app.
fn log_state(state: &Mercury) {
    println!(
        "state: {} | foregrounded {:?}",
        layer_name(&state.layer),
        state.foregrounded
    );
}

const fn layer_name(layer: &Layer) -> &'static str {
    match layer {
        Layer::Home(_) => "home",
        Layer::Nav(_) => "nav",
        Layer::Typing(_) => "typing",
        Layer::InApp(AppLayer::Chrome(_)) => "in-app:chrome",
        Layer::InApp(AppLayer::Other(_)) => "in-app:other",
    }
}

/// The effect loop: read the effect channel and perform each effect.
///
/// The `Emitter` is `!Send` by design, so this future is `!Send`; `main` runs it
/// on the current thread via `join!` and it never crosses a thread.
#[allow(clippy::future_not_send)]
async fn run_effect_loop(
    mut effect_rx: UnboundedReceiver<MercuryEffect>,
    event_tx: UnboundedSender<MercuryEvent>,
    emitter: Emitter,
) {
    while let Some(effect) = effect_rx.recv().await {
        perform_effect(&effect, &event_tx, &emitter);
    }
}

/// Perform one effect: emit keys, foreground an app (reported back as an event,
/// standing in for the OS watcher), or exit.
fn perform_effect(effect: &MercuryEffect, event_tx: &UnboundedSender<MercuryEvent>, emitter: &Emitter) {
    match effect {
        MercuryEffect::Foreground(app) => {
            println!("foreground {app:?}");
            let _ = event_tx.send(foreground(*app));
        }
        MercuryEffect::Emit(ke) => {
            if let Err(e) = emitter.emit(ke.key, ke.press) {
                eprintln!("emit: {e}");
            }
        }
        MercuryEffect::Kill => {
            println!("kill: exiting");
            std::process::exit(0);
        }
    }
}
