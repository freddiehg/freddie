//! A foreground CLI for the Mercury demo.
//!
//! Type one key per line (`n`, `c`, `r`, `space`, `a`, `escape`, ...). Each key
//! is dispatched against the state and its effects are handled by a printing
//! [`EffectHandler`], which describes what it would do rather than touching the
//! real machine, so it is safe to run while other apps are actually foregrounded.
//! After each key it shows the active layer and the keys currently bound (from
//! `bind::accumulate`).
//!
//! `cargo run -p mercury`   (or pipe keys: `printf 'n\nc\nr\n' | cargo run -p mercury`)

use std::io::{self, BufRead};

use mercury::{
    EffectHandler, Key, Layer, Mercury, MercuryEffect, MercuryStruct, MercuryTrigger, drive,
    foreground, key,
};

/// Prints each effect. Foregrounding an app is where the demo would call the OS;
/// here it just reports success and lets the follow-up foreground event through.
struct Printer;

impl EffectHandler for Printer {
    fn handle(&mut self, effect: &MercuryEffect, state: &mut Mercury) -> Vec<MercuryEffect> {
        match effect {
            MercuryEffect::Foreground(app) => {
                println!("  foreground {app:?}");
                // A real build would open the app here and only continue on
                // success; the demo always succeeds. See the `Recorder` in the
                // tests for the failure path (no app, so no follow-up).
                state.handle(&foreground(*app)).unwrap_or_default()
            }
            MercuryEffect::Type(s) => {
                println!("  type {s}");
                Vec::new()
            }
            MercuryEffect::Command(k) => {
                println!("  send cmd+{k}");
                Vec::new()
            }
        }
    }
}

fn main() {
    let mut state = Mercury::default();
    let mut printer = Printer;
    println!("mercury — one key per line (Ctrl-D to quit)");
    print_status(&state);

    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        // The model's keys are `&'static str`; leak the input to match. Fine for
        // a short-lived CLI.
        let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
        println!("> {name}");
        drive(&mut state, &key(name), &mut printer);
        print_status(&state);
    }
}

fn print_status(state: &Mercury) {
    let mut keys: Vec<&str> = bind::accumulate::<MercuryStruct, _>(state)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| match t {
            MercuryTrigger::Key(Key(k)) => Some(k),
            MercuryTrigger::Foregrounded(_) => None,
        })
        .collect();
    keys.sort_unstable();
    println!(
        "[{} | foregrounded {:?}] keys: {}",
        layer_name(&state.layer),
        state.foregrounded,
        keys.join(" ")
    );
}

const fn layer_name(layer: &Layer) -> &'static str {
    match layer {
        Layer::Home(_) => "home",
        Layer::Nav(_) => "nav",
        Layer::Typing(_) => "typing",
        Layer::InApp(_) => "in-app",
    }
}
