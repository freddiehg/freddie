//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;
pub mod drop_guard;
pub mod sequence;
pub mod timer;

pub use always_equal::AlwaysEqual;
pub use drop_guard::{DropGuard, drop_guard};
pub use sequence::{KeySequence, KeySequenceOutcome};
pub use timer::{
    TimerEffect, TimerFired, TimerGuard, TimerId, TimerTrigger, timer_effect_and_guard,
};
