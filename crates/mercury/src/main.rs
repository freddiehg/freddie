//! Mercury's runnable v1: the real event loop, minus the macOS keyboard FFI.
//!
//! Three stages, each its own loop, joined by two `tokio` channels:
//!
//! - a source forwards events into the event channel (v1: stdin, one key per
//!   line; the real build: a `CGEventTap` thread),
//! - the event loop reads the event channel and dispatches each event (mutating
//!   state, producing effects) into the effect channel,
//! - the effect loop reads the effect channel and performs each effect (v1:
//!   prints; the real build: OS key synthesis and app activation).
//!
//! Killing is an effect. A separate killswitch task enqueues `Kill` after 5s and
//! the effect loop performs it by exiting cleanly; after 10s the killswitch
//! hard-exits directly, the backstop for when the effect loop is wedged. Either
//! way a run ends on its own.
//!
//! `cargo run -p mercury` (type keys, or pipe them: `printf 'n\nc\nr\n' | cargo run -p mercury`).

use std::io::{self, BufRead};
use std::time::Duration;

use mercury::{Mercury, MercuryEffect, MercuryEvent, foreground, key};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    spawn_stdin_source(event_tx.clone());
    spawn_killswitch(effect_tx.clone());
    tokio::spawn(run_effect_loop(effect_rx, event_tx.clone()));

    run_event_loop(Mercury::default(), event_rx, effect_tx).await;
}

/// Source: read stdin on its own thread and forward one key event per line into
/// the event channel. Mirrors the real keyboard source, which forwards from its
/// `CGEventTap` run-loop thread.
fn spawn_stdin_source(event_tx: UnboundedSender<MercuryEvent>) {
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let name = line.trim();
            if name.is_empty() {
                continue;
            }
            // The model's keys are `&'static str`; leak the input to match. Fine
            // for a short-lived CLI.
            let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
            if event_tx.send(key(name)).is_err() {
                break; // the event loop is gone
            }
        }
    });
}

/// Dev killswitch: a `Kill` effect after 5s (performed by the effect loop, a
/// clean exit), then a hard exit after 10s if that never happened.
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
        let _ = effect_tx.send(effect); // the effect loop performs it
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

/// Perform one effect. v1 prints; `Kill` exits; a `Foreground` also feeds the
/// follow-up event back, standing in for the OS foreground watcher.
fn perform_effect(effect: &MercuryEffect, event_tx: &UnboundedSender<MercuryEvent>) {
    match effect {
        MercuryEffect::Foreground(app) => {
            println!("  foreground {app:?}");
            // v1 has no real watcher; report the app came up ourselves.
            let _ = event_tx.send(foreground(*app));
        }
        MercuryEffect::Type(s) => println!("  type {s}"),
        MercuryEffect::Command(k) => println!("  send cmd+{k}"),
        MercuryEffect::Kill => {
            println!("kill: exiting");
            std::process::exit(0);
        }
    }
}
