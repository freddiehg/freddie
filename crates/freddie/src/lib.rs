//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;
pub mod timer;

pub use always_equal::AlwaysEqual;
pub use timer::{TimerEffect, TimerGuard, timer_effect_and_guard};
