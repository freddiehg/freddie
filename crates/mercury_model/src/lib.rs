//! mercury's model: a pure function of state and event.
//!
//! This crate cannot call macOS. No platform crate is in its dependency graph, the workspace
//! forbids `unsafe`, and clippy denies std's own OS surface (processes, threads, clocks,
//! files, sockets). `cargo check -p mercury_model` is the proof: if it compiles, dispatch
//! reads nothing but `(state, event)`. The one sanctioned impurity is timer-guard minting
//! through `freddie::timer_effect_and_guard`, which is channel construction, not an OS call.

pub use freddie_keys::{Key, KeyEvent, KeyPress, ModifierFlags, PressType};

mod effect;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{Chord, MercuryEffect, UrlPart};
pub use freddie_windows_types::{Pid, Placement};
pub use model::{MercuryEvent, MercuryStruct, MercuryTrigger};
pub use sources::{
    AnyKey, App, FocusLanded, FocusRead, ForegroundEvent, Foregrounded, FrameLanded, FrameRead,
    Quit, Site, TabEvent, Tabbed, WindowEvent, Windowed, host,
};
pub use state::{
    AndReturnHome, AppData, AppLayer, ChromeApp, ClaudeAiSite, ForegroundedApp, ForegroundedChrome,
    FrontApp, GhosttyApp, HomeLayer, JK_TIMEOUT, Layer, Mercury, NavLayer, OVERLAY_DWELL,
    PLACEMENT_SETTLE, RETURN_TO_HOME_TIMEOUT, ResizeLayer, ReturnHomeLayers, SiteData, SiteLayer,
    TypingLayer, Windows, focus_read, foreground, frame_read, key, quit_event, tab,
};
