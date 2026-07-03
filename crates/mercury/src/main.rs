//! Mercury's runnable v1: grab the keyboard and drive the model.
//!
//! `freddie_keyboard::run` swallows every key and hands it to the model. Three
//! loops over two tokio channels: the keyboard source forwards key-downs; the
//! event loop dispatches (mutating state, producing effects); the effect loop
//! performs them (re-emitting keys, and a `Foreground` reports itself back,
//! standing in for the OS foreground watcher). `escape` exits from the capture
//! callback (the way out of a full hijack); a 5-second timer is the backstop
//! (10-second hard exit).
//!
//! On macOS this needs Accessibility (and Input Monitoring) granted to the
//! terminal (or the built app).
//!
//! `cargo run -p mercury`

use std::time::Duration;

use mercury::{AppLayer, Keyboard, Layer, Mercury, MercuryEffect, MercuryEvent, foreground, key};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    spawn_keyboard_source(event_tx.clone());
    spawn_killswitch(effect_tx.clone());
    tokio::spawn(run_effect_loop(effect_rx, event_tx.clone()));

    println!("mercury: hijacking the keyboard; escape quits (5s backstop)");
    run_event_loop(Mercury::default(), event_rx, effect_tx).await;
}

/// Source: grab the keyboard on its own thread, swallowing every key and
/// forwarding each key-down to the model. `escape` exits the process, which is the
/// one way out of a full hijack and does not depend on the model or the channel.
fn spawn_keyboard_source(event_tx: UnboundedSender<MercuryEvent>) {
    std::thread::spawn(move || {
        let grabbed = freddie_keyboard::run(move |ev| {
            if ev.key == Keyboard::Escape {
                std::process::exit(0);
            }
            if ev.down {
                let _ = event_tx.send(key(ev.key));
            }
        });
        if let Err(e) = grabbed {
            eprintln!("keyboard: {e}"); // usually Accessibility is not granted
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
        MercuryEffect::Type(k) => {
            if let Err(e) = freddie_keyboard::emit(*k) {
                eprintln!("emit: {e}");
            }
        }
        MercuryEffect::Command(k) => {
            if let Err(e) = freddie_keyboard::emit_chord(&[Keyboard::MetaLeft], *k) {
                eprintln!("emit: {e}");
            }
        }
        MercuryEffect::Kill => {
            println!("kill: exiting");
            std::process::exit(0);
        }
    }
}
