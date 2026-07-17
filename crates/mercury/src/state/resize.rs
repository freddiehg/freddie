use bind::Bind;
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::LayerPath;

/// The resize layer: the arrows place the focused window and return home. Like nav, a one-shot
/// chooser.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => maximize,
    Key::LeftArrow.down() => left_half,
    Key::RightArrow.down() => right_half,
)]
pub struct ResizeLayer {}

impl ResizeLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}
