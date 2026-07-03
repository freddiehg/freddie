//! Keyboard capture and emission for freddie, over the `freddie_keys` vocabulary.
//!
//! An OS-agnostic API ([`run`], [`emit`], [`emit_chord`]) with the OS-specific
//! work behind it. v1 ships one backend, macOS on `core-graphics`; Linux and
//! Windows backends slot in behind `cfg` later. Nothing above this crate sees a
//! platform type.
//!
//! On macOS `run` needs Accessibility (and Input Monitoring) granted to whatever
//! launches the binary.

use std::fmt;

pub use freddie_keys::{KeyEvent, Keyboard};

mod sys;

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
    /// The event source could not be created.
    Source,
    /// The key event could not be built or posted.
    Post,
    /// This key has no code on this OS, so it cannot be emitted.
    Unmappable(Keyboard),
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source => f.write_str("could not create the event source"),
            Self::Post => f.write_str("could not build or post the key event"),
            Self::Unmappable(key) => write!(f, "{key:?} has no key code on this OS"),
        }
    }
}

impl std::error::Error for EmitError {}

/// Intercept the keyboard, swallowing every key and handing each to `on_key`.
///
/// Blocks the calling thread (the OS delivers keys on it), so run it on its own
/// thread. Nothing reaches other apps unless a key is re-emitted with [`emit`].
///
/// # Errors
///
/// Returns [`CaptureError`] if the interceptor cannot start.
pub fn run(on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<(), CaptureError> {
    sys::run(on_key)
}

/// Emit a key, pressing then releasing it, tagged so a running [`run`] ignores it.
///
/// # Errors
///
/// Returns [`EmitError`] if the key has no code on this OS or could not be posted.
pub fn emit(key: Keyboard) -> Result<(), EmitError> {
    sys::emit(key)
}

/// Emit `key` with `mods` held around it (a chord like cmd+r), tagged like [`emit`].
///
/// # Errors
///
/// Returns [`EmitError`] if a key has no code on this OS or could not be posted.
pub fn emit_chord(mods: &[Keyboard], key: Keyboard) -> Result<(), EmitError> {
    sys::emit_chord(mods, key)
}
