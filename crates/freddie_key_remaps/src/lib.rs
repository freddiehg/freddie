//! Shareable pure keyboard remaps: dual-role state machines and flag rewrites over [`freddie_keys`].
//!
//! No effects, no timers, no bind. A consumer feeds [`freddie_keys::KeyEvent`]s, gets back events
//! to emit and flags to stamp. State lives in the struct the consumer owns on its root model.
//!
//! Ordered timed chords (`jk`) live in `freddie::KeySequence`, not here.

mod alone_or_modifier;

pub use alone_or_modifier::AloneOrModifier;
