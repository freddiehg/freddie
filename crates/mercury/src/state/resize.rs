use bind::{Bind, and};
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::ReturnHomeLayersPath;

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser, so it idles back home too.
/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/resize.txt");

#[derive(Bind, Debug)]
#[node(parent_path = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => go_home,
    Key::KeyT.down() => enter_typing,
    Key::UpArrow.down() => and!(maximize, go_home),
    Key::LeftArrow.down() => and!(left_half, go_home),
    Key::RightArrow.down() => and!(right_half, go_home),
    Key::KeyR.down() => and!(restore, go_home),
)]
pub struct ResizeLayer;

impl ResizeLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
