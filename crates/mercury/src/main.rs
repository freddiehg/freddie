//! A foreground CLI for the Mercury demo.
//!
//! Type one key per line (`n`, `c`, `r`, `space`, `a`, `escape`, ...). Each key
//! is run through the little event loop below: dispatch it, print the effects,
//! and push a `Foreground` effect's follow-up event (assuming the app opened).
//! Nothing touches the real machine, so it is safe to run while other apps are
//! foregrounded. After each key it shows the active layer and the keys currently
//! bound (from `bind::accumulate`).
//!
//! `cargo run -p mercury`   (or pipe keys: `printf 'n\nc\nr\n' | cargo run -p mercury`)

use std::collections::VecDeque;
use std::io::{self, BufRead};

use mercury::{Key, Layer, Mercury, MercuryEffect, MercuryEvent, MercuryStruct, MercuryTrigger, foreground, key};

fn main() {
    let mut state = Mercury::default();
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

        // The event loop, written inline: drain the queue, and a `Foreground`
        // effect pushes its follow-up event (the app coming up).
        let mut queue: VecDeque<MercuryEvent> = VecDeque::from([key(name)]);
        while let Some(event) = queue.pop_front() {
            if let Some(output) = state.handle(&event) {
                for effect in output {
                    println!("  {}", render(&effect));
                    if let MercuryEffect::Foreground(app) = effect {
                        queue.push_front(foreground(app));
                    }
                }
            }
        }
        print_status(&state);
    }
}

fn render(effect: &MercuryEffect) -> String {
    match effect {
        MercuryEffect::Foreground(app) => format!("foreground {app:?}"),
        MercuryEffect::Type(s) => format!("type {s}"),
        MercuryEffect::Command(k) => format!("send cmd+{k}"),
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
