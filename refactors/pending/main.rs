//! Illustrative usage sketch for phantom-kit-2. This does NOT compile: the
//! `phantom_kit` crate, the derive macro, and the `Event`/`Key` types do not
//! exist yet. The point is to show what *using* the library would feel like.
//!
//! Per the design notes, the event type is deliberately not built out here. We
//! only reference the shape we'd pattern-match on:
//!
//!     enum Event {
//!         Key(Key),     // a physical key, e.g. Key::Esc, Key::Space, Key::G
//!         Custom(u8),   // an arbitrary user-defined event, e.g. "event 1"
//!     }
//!
//! Run it as a background process for v1:
//!
//!     cargo run --release &      # start
//!     pkill phantom-kit          # stop
//!
//! (A real `start`/`stop` CLI with a pid file is described in the plan doc.)

use phantom_kit::Phantom;
use tokio_stream::StreamExt;

// ---------------------------------------------------------------------------
// The state machine.
//
// These are not "layers" in any meaningful sense. Each variant just means "we
// are in the situation modeled by this struct," and the struct can hold
// arbitrary, non-trivial data (see `Type` below). Keyboard modes are one use of
// this, not the model.
//
// `#[derive(Phantom)]` goes on the union of states. Each variant wraps its own
// struct, which holds that state's data. The derive generates the top-level
// dispatch (`Keyboard::handle`) that matches on the current variant and calls
// the matching struct's `handle`, plus `Keyboard::events()` which aggregates the
// current variant's event sources. We write the per-state behavior by hand; the
// derive only wires the mechanical parts.
// ---------------------------------------------------------------------------

// Q: how are the Event/Effect types bound to the machine? Attribute args as
//    shown, generic params on the enum, or associated types on a trait?
// Q: how does the derive find each variant's behavior? By convention (an
//    inherent `fn handle` on the kind struct, as below), or by requiring the
//    kind struct to impl a library trait (`Layer`)? Convention = less
//    boilerplate; trait = better errors and discoverability.
#[derive(Phantom, Clone)]
#[phantom(event = Event, effect = Effect)]
enum Keyboard {
    Nav(Nav),
    Type(Type),
}

#[derive(Clone, Default)]
struct Nav;

#[derive(Clone, Default)]
struct Type;

// Effects are a *user-defined* type: the core returns them as data, the caller
// (see `run_effect` below) decides how to perform them. The library never
// performs I/O itself.
enum Effect {
    /// Emit the key to the OS as-is (typing).
    PassThrough(Key),
    /// Launch (or focus) an application.
    OpenApp { app: &'static str, foreground: bool },
}

// ---------------------------------------------------------------------------
// Per-layer behavior. Each kind struct gets an inherent `handle` that consumes
// the current layer and returns (next layer, effects). Effects are NOT executed
// here; they're returned for the caller to run. `Vec` is fine; most steps emit
// zero or one effect.
// ---------------------------------------------------------------------------

// Q: the return type names the whole `Keyboard` enum to express a transition,
//    so a layer has to know its siblings. Acceptable, but is there a nicer
//    surface (a generated `Transition` helper, or returning `impl Into<Keyboard>`)
//    so `Nav` doesn't hard-reference `Type`?
// Q: `Vec<Effect>` allocates every step even when empty. Worth a SmallVec /
//    ArrayVec, or an enum `Effects { None, One(Effect), Many(Vec<_>) }`?
// Q: where do on_enter / on_exit hooks live? This example has none. If entering
//    a layer should fire an effect (notify SwiftBar, swap keymap), is that an
//    effect returned by `handle`, or a separate hook the driver runs by diffing
//    old vs new state? Nothing here exercises it yet.
impl Nav {
    fn handle(self, event: Event) -> (Keyboard, Vec<Effect>) {
        match event {
            // space -> Type
            Event::Key(Key::Space) => (Keyboard::Type(Type), vec![]),

            // g -> open Chrome in the foreground; stay in Nav
            Event::Key(Key::G) => (
                Keyboard::Nav(self),
                vec![Effect::OpenApp { app: "Google Chrome", foreground: true }],
            ),

            // "event 1" -> open Ghostty; stay in Nav
            Event::Custom(1) => (
                Keyboard::Nav(self),
                vec![Effect::OpenApp { app: "Ghostty", foreground: true }],
            ),

            // Nav swallows everything else (it is not a typing layer).
            _ => (Keyboard::Nav(self), vec![]),
        }
    }
}

impl Type {
    fn handle(self, event: Event) -> (Keyboard, Vec<Effect>) {
        match event {
            // Esc is the one escape hatch: back to Nav, emit nothing.
            // Q: the target variant is constructed fresh (`Nav`). When the kind
            //    struct holds state, is the target Default-constructed, or can a
            //    transition carry fields over from the old layer? We own `self`
            //    here, so carry-over is possible; what's the ergonomic default?
            Event::Key(Key::Esc) => (Keyboard::Nav(Nav), vec![]),

            // Everything else passes through as a normal keystroke. A real
            // build would constrain `Key` to the supported set; a limited set
            // is sufficient to show the shape.
            Event::Key(k) => (Keyboard::Type(self), vec![Effect::PassThrough(k)]),

            // Non-key events are ignored while typing.
            _ => (Keyboard::Type(self), vec![]),
        }
    }
}

// ---------------------------------------------------------------------------
// The driver loop. This is the shape from the plan: recompute the current
// state's own event sources each iteration, select across the user-wired
// sources and the state sources, fold the event into the state, then run the
// returned effects. The loop itself is plain; nothing is hidden in a runtime.
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let mut state = Keyboard::Nav(Nav);

    // External event sources are wired up by *us*, not the library. For v1
    // these are just streams we feed from wherever (an OS key tap, a socket,
    // a hardware button). Their construction is out of scope here.
    // Q: does the OS key source belong in the library at all, or is it 100%
    //    the user's to provide (sans-I/O core)? Shown here as `phantom_kit::*`
    //    for brevity, but the boundary says the user wires real sources.
    let mut keys = phantom_kit::os_key_source(); // Stream<Item = Event>
    let mut buttons = phantom_kit::custom_source(); // Stream<Item = Event> ("event 1", ...)

    // Q: should the library ship this whole loop as a `run(state, sources)`
    //    driver, or do users hand-write the select? Writing it out (as here)
    //    keeps the loop visible and unmagical, but it's boilerplate every user
    //    repeats. Maybe ship `run` and document this expansion.
    loop {
        // Sources this *layer* cares about (e.g. timers), recomputed each turn
        // so subscription tracks the current state. Empty in this example since
        // neither layer arms a timer.
        // Q: what concrete type does `events()` return? A single merged
        //    `Stream<Item = Event>`? How are multiple per-state sources merged,
        //    and how do we keep timers from restarting on rebuild (absolute
        //    deadlines stored in state, per the plan)?
        let mut state_events = state.events();

        // Q: `state.events()` borrows `state`, but `state.handle(...)` below
        //    consumes it. The borrow must end before the move. Works here
        //    (the stream yields, then we drop it), but the ownership dance is
        //    real and the `run` driver would need to get it right.
        let event = tokio::select! {
            Some(e) = keys.next() => e,
            Some(e) = buttons.next() => e,
            Some(e) = state_events.next() => e,
        };

        let (next, effects) = state.handle(event);
        for effect in effects {
            run_effect(effect);
        }
        state = next;
    }
}

/// The caller interprets effects. This is the only place that touches the world.
fn run_effect(effect: Effect) {
    match effect {
        Effect::PassThrough(key) => phantom_kit::emit_key(key),
        Effect::OpenApp { app, foreground } => {
            let mut cmd = std::process::Command::new("open");
            if !foreground {
                cmd.arg("-g");
            }
            let _ = cmd.args(["-a", app]).spawn();
        }
    }
}
