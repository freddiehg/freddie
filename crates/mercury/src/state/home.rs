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
    Key::Escape.down() => go_home,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyN.down() => enter_nav,
    Key::KeyR.down() => enter_resize,
    Key::KeyT.down() => enter_typing,
    Key::KeyI.down() => enter_inapp,
    Key::KeyU.down() => enter_site,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer;

impl HomeLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
