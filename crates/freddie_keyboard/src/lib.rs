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

/// What can go wrong intercepting or emitting keys.
#[derive(Debug)]
pub enum Error {
    /// Could not start the interceptor (usually missing permission).
    Grab(rdev::GrabError),
    /// Could not emit a key.
    Simulate(rdev::SimulateError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grab(e) => write!(f, "could not intercept the keyboard: {e:?}"),
            Self::Simulate(e) => write!(f, "could not emit a key: {e:?}"),
        }
    }
}

impl std::error::Error for Error {}

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
/// Returns [`Error::Grab`] if the interceptor cannot start, which on macOS
/// usually means Accessibility or Input Monitoring is not granted.
pub fn run(on_key: impl Fn(KeyEvent) + 'static) -> Result<(), Error> {
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
    .map_err(Error::Grab)
}

/// Emit a key, pressing and releasing it, as if typed. Marked so a running
/// [`run`] interceptor lets it through instead of re-handling it.
///
/// # Errors
///
/// Returns [`Error::Simulate`] if the key could not be posted.
pub fn emit(key: Key) -> Result<(), Error> {
    SYNTHETIC.fetch_add(2, Ordering::SeqCst); // one for the press, one for the release
    rdev::simulate(&rdev::EventType::KeyPress(key)).map_err(Error::Simulate)?;
    rdev::simulate(&rdev::EventType::KeyRelease(key)).map_err(Error::Simulate)?;
    Ok(())
}
