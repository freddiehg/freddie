//! Mercury's runnable v1: watch the keyboard (no hijack) and drive the model.
//!
//! `freddie_keyboard::listen` observes keys without swallowing them, so this is
//! safe to run: every key still works normally, and a copy drives Mercury's
//! layers. Three loops over two tokio channels: the keyboard source forwards key
//! events; the event loop dispatches (mutating state, producing effects); the
//! effect loop performs them (v1 prints, and a `Foreground` reports itself back,
//! standing in for the OS foreground watcher). `escape` quits; a 5-second timer
//! also quits (10-second hard backstop).
//!
//! On macOS this needs Input Monitoring granted to the terminal (or the built
//! app).
//!
//! `cargo run -p mercury`

use std::time::Duration;

use mercury::{AppLayer, Layer, Mercury, MercuryEffect, MercuryEvent, foreground, key};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    spawn_keyboard_source(event_tx.clone());
    spawn_killswitch(effect_tx.clone());
    tokio::spawn(run_effect_loop(effect_rx, event_tx.clone()));

    println!("mercury: watching the keyboard (not hijacked); escape or 5s quits");
    run_event_loop(Mercury::default(), event_rx, effect_tx).await;
}

/// Source: watch the keyboard on its own thread and forward each key-down as a
/// Mercury key event. Observing only (no swallow), so the keyboard still works.
fn spawn_keyboard_source(event_tx: UnboundedSender<MercuryEvent>) {
    std::thread::spawn(move || {
        let listened = freddie_keyboard::listen(move |ev| {
            if !ev.press {
                return; // v1 dispatches on key-down
            }
            if let Some(name) = freddie_keyboard::name(ev.key) {
                let _ = event_tx.send(key(name));
            }
        });
        if let Err(e) = listened {
            eprintln!("keyboard: {e}"); // usually Input Monitoring is not granted
        }
    });
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
async fn run_effect_loop(
    mut effect_rx: UnboundedReceiver<MercuryEffect>,
    event_tx: UnboundedSender<MercuryEvent>,
) {
    while let Some(effect) = effect_rx.recv().await {
        perform_effect(&effect, &event_tx);
    }
}

/// Perform one effect. v1 prints; `Kill` exits; a `Foreground` also reports the
/// app back as an event, standing in for the OS foreground watcher.
fn perform_effect(effect: &MercuryEffect, event_tx: &UnboundedSender<MercuryEvent>) {
    match effect {
        MercuryEffect::Foreground(app) => {
            println!("foreground {app:?}");
            let _ = event_tx.send(foreground(*app));
        }
        MercuryEffect::Type(s) => println!("type {s}"),
        MercuryEffect::Command(k) => println!("send cmd+{k}"),
        MercuryEffect::Kill => {
            println!("kill: exiting");
            std::process::exit(0);
        }
    }
}
