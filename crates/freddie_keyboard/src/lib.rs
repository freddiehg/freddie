//! Keyboard interception and emission for freddie, on macOS via `core-graphics`.
//!
//! [`intercept`] grabs the keyboard and hands back an [`Interceptor`] and an
//! [`Emitter`]. The interceptor's callback decides each key and returns what it
//! becomes (`Some(same)` passes, `Some(other)` remaps, `None` drops); the emitter
//! synthesizes keys not tied to an intercepted event. See
//! `refactors/past/keyboard-capture.md`.

use std::fmt;

pub use freddie_keys::{Key, KeyEvent, PressType};

mod sys;
pub use sys::{Emitter, Interceptor, intercept};

/// The keyboard could not be intercepted. On macOS this usually means
/// Accessibility (or Input Monitoring) is not granted.
#[derive(Debug)]
pub struct CaptureError;

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not intercept the keyboard (is Accessibility granted?)")
    }
}

impl std::error::Error for CaptureError {}

/// A key could not be emitted.
#[derive(Debug)]
pub enum EmitError {
    /// The key has no code on this OS, so it cannot be emitted.
    Unmappable(Key),
    /// The OS refused to build or post the event.
    Post,
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmappable(key) => write!(f, "{key:?} has no key code on this OS"),
            Self::Post => f.write_str("could not build or post the key event"),
        }
    }
}

impl std::error::Error for EmitError {}
