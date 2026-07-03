//! Keyboard interception and emission for freddie.
//!
//! [`run`] intercepts the keyboard: every key is swallowed at the OS level and
//! handed to your callback, so nothing reaches other apps unless you re-emit it
//! with [`emit`]. This wraps `rdev`, so the platform specifics (a `CGEventTap` on
//! macOS, hooks elsewhere) stay here and the rest of freddie never sees them.
//!
//! On macOS this needs Accessibility (and Input Monitoring) granted to whatever
//! launches the binary.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use rdev::Key;

/// A key going down (`press`) or coming up.
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent {
    /// Which physical key.
    pub key: Key,
    /// `true` on key-down, `false` on key-up.
    pub press: bool,
}

/// The interceptor could not start, usually because Accessibility (or Input
/// Monitoring) is not granted. The failure of [`run`], and only [`run`].
#[derive(Debug)]
pub struct GrabError(rdev::GrabError);

impl fmt::Display for GrabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not intercept the keyboard: {:?}", self.0)
    }
}

impl std::error::Error for GrabError {}

/// The observer could not start, usually because Input Monitoring is not
/// granted. The failure of [`listen`], and only [`listen`].
#[derive(Debug)]
pub struct ListenError(rdev::ListenError);

impl fmt::Display for ListenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not observe the keyboard: {:?}", self.0)
    }
}

impl std::error::Error for ListenError {}

/// A key could not be emitted. The failure of [`emit`], and only [`emit`].
#[derive(Debug)]
pub struct SimulateError(rdev::SimulateError);

impl fmt::Display for SimulateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not emit a key: {:?}", self.0)
    }
}

impl std::error::Error for SimulateError {}

// Keys we emit are counted here so [`run`] lets them through instead of feeding
// our own output back into the callback. rdev does not tag synthetic events, so
// this stands in for the `kCGEventSourceUserData` marker a raw tap would use.
static SYNTHETIC: AtomicUsize = AtomicUsize::new(0);

/// Intercept the keyboard, swallowing every key and handing it to `on_key`.
///
/// Blocks the calling thread (the OS delivers keys on it), so run it on its own
/// thread. Nothing reaches other apps unless a key is re-emitted with [`emit`].
///
/// # Errors
///
/// Returns [`GrabError`] if the interceptor cannot start, which on macOS
/// usually means Accessibility or Input Monitoring is not granted.
pub fn run(on_key: impl Fn(KeyEvent) + 'static) -> Result<(), GrabError> {
    rdev::grab(move |event| {
        let (key, press) = match event.event_type {
            rdev::EventType::KeyPress(key) => (key, true),
            rdev::EventType::KeyRelease(key) => (key, false),
            _ => return Some(event), // not a key; leave it untouched
        };
        // Our own emitted key: let it through once and stop counting it.
        if SYNTHETIC.load(Ordering::SeqCst) > 0 {
            SYNTHETIC.fetch_sub(1, Ordering::SeqCst);
            return Some(event);
        }
        on_key(KeyEvent { key, press });
        None // swallow real keys; the consumer re-emits what it wants
    })
    .map_err(GrabError)
}

/// Observe the keyboard without swallowing anything.
///
/// Every key still reaches other apps, and a copy is handed to `on_key`. Safe to
/// run (no hijack), the v1 path. Blocks the calling thread, so run it on its own.
///
/// # Errors
///
/// Returns [`ListenError`] if listening cannot start, which on macOS usually
/// means Input Monitoring is not granted.
pub fn listen(on_key: impl Fn(KeyEvent) + 'static) -> Result<(), ListenError> {
    rdev::listen(move |event| {
        let (key, press) = match event.event_type {
            rdev::EventType::KeyPress(key) => (key, true),
            rdev::EventType::KeyRelease(key) => (key, false),
            _ => return,
        };
        on_key(KeyEvent { key, press });
    })
    .map_err(ListenError)
}

/// Emit a key, pressing and releasing it, as if typed. Marked so a running
/// [`run`] interceptor lets it through instead of re-handling it.
///
/// # Errors
///
/// Returns [`SimulateError`] if the key could not be posted.
pub fn emit(key: Key) -> Result<(), SimulateError> {
    SYNTHETIC.fetch_add(2, Ordering::SeqCst); // one for the press, one for the release
    rdev::simulate(&rdev::EventType::KeyPress(key)).map_err(SimulateError)?;
    rdev::simulate(&rdev::EventType::KeyRelease(key)).map_err(SimulateError)?;
    Ok(())
}
