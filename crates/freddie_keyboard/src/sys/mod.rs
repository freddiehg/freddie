//! Backend selection. macOS on `core-graphics` is the only one so far.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Emitter, Held, Interceptor, intercept};

#[cfg(not(target_os = "macos"))]
compile_error!("freddie_keyboard only has a macOS backend so far");
