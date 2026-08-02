use bind::{Bind, if_not_invalidated};
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
    Key::KeyT.down() => if_not_invalidated(enter_typing),
    Key::UpArrow.down() => if_not_invalidated(maximize),
    Key::LeftArrow.down() => if_not_invalidated(left_half),
    Key::RightArrow.down() => if_not_invalidated(right_half),
    Key::KeyR.down() => if_not_invalidated(restore),
)]
pub struct ResizeLayer;

impl ResizeLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
