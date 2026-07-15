//! Mercury: a small, runnable demo of freddie (laserbeam + bind).
//!
//! It models a layered keyboard remapper as a pure state tree; it defines no traits of its
//! own. The state is an outer [`Mercury`] holding the currently foregrounded app and a
//! [`Layer`] it resolves into:
//!
//! - [`HomeLayer`] (the default): `n` enters nav, `t` enters typing, `i` enters the in-app
//!   layer for whatever app is foregrounded.
//! - [`NavLayer`]: `c`/`g`/`z` foreground Chrome/Ghostty/Zed and go back to home. Nav is a
//!   one-shot chooser, so `n c i r` navigates to Chrome and refreshes it.
//! - [`ResizeLayer`] (`r` from home): the arrows place the focused window, up to maximize and
//!   left and right to the halves, then it goes back to home.
//! - [`TypingLayer`]: `escape` goes home, any other key passes through.
//! - [`AppLayer`] (in-app): it stores NO app. A derived child fn reads `root.foregrounded` on
//!   every dispatch and builds the app's level from it, so there is one copy of the
//!   foregrounded app and nothing to keep in sync. [`ChromeApp`] binds `r` to a refresh;
//!   [`GhosttyApp`] binds `j`/`k` to tmux's previous and next window and `1`-`0` to windows one
//!   through ten. An app with no bindings gets no level at all.
//!
//! A layer stays only if its actions make sense to do repeatedly. Walking panes and refreshing
//! a page do, so the in-app layers stay put. Choosing an app or a window placement does not, so
//! nav and resize are one-shot choosers that return home.
//!
//! `escape` goes back to the home layer from every sub-layer, and is a no-op in home (it
//! re-enters home). Typing binds it explicitly so its catch-all does not shadow the go-home
//! binding. From home, `q` quits, so `escape` then `q` is the way out of any layer.
//!
//! A foreground event records which app is frontmost at the root. Handlers either mutate the
//! state through the path they are handed (the layer transitions) or return inert
//! [`MercuryEffect`]s. Dispatch is opaque to what an effect does; performing effects is the
//! caller's job (see the CLI and the tests).
//!
//! The code is split by concern: [`sources`] and [`effect`] are the domain types, [`model`] is
//! the unified trigger/event/marker, [`state`] is the state nodes and their bindings, and
//! `handlers` holds the handler functions, one module per layer.
//!
//! Run it with `cargo run -p mercury`, or the tests with `cargo test -p mercury`.

pub use freddie_keys::{Key, KeyEvent, KeyPress, PressType};

mod effect;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{MercuryEffect, Placement};
pub use model::{MercuryEvent, MercuryStruct, MercuryTrigger};
pub use sources::{AnyKey, App, ForegroundEvent, Foregrounded};
pub use state::{
    AppData, AppLayer, ChromeApp, GhosttyApp, HomeLayer, Layer, Mercury, NavLayer, ResizeLayer,
    TypingLayer, foreground, key,
};
