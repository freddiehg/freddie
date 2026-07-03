//! Backend selection. Each OS backend provides `run`, `emit`, and `emit_chord`
//! over `freddie_keys::Keyboard`; the public API in the crate root delegates here.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{emit, emit_chord, run};

#[cfg(not(target_os = "macos"))]
compile_error!("freddie_keyboard has no keyboard backend for this OS yet (macOS only so far)");
