//! Mercury's CLI: stand up the event and effect loops and a kill timer, then run.
//!
//! It does not hijack the keyboard yet. The keyboard source is a stub (the real
//! `CGEventTap`, and the foreground watcher, go there later), so nothing feeds the
//! event channel and the run just holds the loops open. A 5-second timer sends a
//! `Kill` effect, which the effect loop performs as a clean exit; a 10-second
//! timer hard-exits, the backstop.
//!
//! `cargo run -p mercury`

use std::time::Duration;

use mercury::{Mercury, MercuryEffect, MercuryEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    spawn_keyboard_source(event_tx);
    spawn_killswitch(effect_tx.clone());
    tokio::spawn(run_effect_loop(effect_rx));

    println!("mercury: loops up, keyboard not hijacked; killing in 5s");
    run_event_loop(Mercury::default(), event_rx, effect_tx).await;
}

/// The keyboard source. TODO: a real `CGEventTap` run loop goes here (and the
/// `NSWorkspace` foreground watcher alongside it). It must not hijack the keyboard
/// yet, so for now the thread just holds the sender open and forwards nothing.
fn spawn_keyboard_source(event_tx: UnboundedSender<MercuryEvent>) {
    std::thread::spawn(move || {
        let _event_tx = event_tx;
        std::thread::park();
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
        return;
    };
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}

/// The effect loop: read the effect channel and perform each effect.
async fn run_effect_loop(mut effect_rx: UnboundedReceiver<MercuryEffect>) {
    while let Some(effect) = effect_rx.recv().await {
        perform_effect(&effect);
    }
}

/// Perform one effect. v1 prints; `Kill` exits. The real build synthesizes keys
/// and activates apps here.
fn perform_effect(effect: &MercuryEffect) {
    match effect {
        MercuryEffect::Foreground(app) => println!("foreground {app:?}"),
        MercuryEffect::Type(s) => println!("type {s}"),
        MercuryEffect::Command(k) => println!("send cmd+{k}"),
        MercuryEffect::Kill => {
            println!("kill: exiting");
            std::process::exit(0);
        }
    }
}
