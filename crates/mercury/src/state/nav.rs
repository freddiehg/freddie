use bind::Bind;
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::LayerPath;

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyC.down() => open_chrome,
    Key::KeyF.down() => open_finder,
    Key::KeyG.down() => open_ghostty,
    Key::KeyZ.down() => open_zed,
)]
pub struct NavLayer {}

impl NavLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}
