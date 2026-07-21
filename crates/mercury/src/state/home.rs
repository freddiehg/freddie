use bind::Bind;
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::LayerPath;

/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/home.txt");

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyU.down() => to_site,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer;

impl HomeLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
