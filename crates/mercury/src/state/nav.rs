use bind::{Bind, and};
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::ReturnHomeLayersPath;

/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/nav.txt");

#[derive(Bind, Debug)]
#[node(parent_path = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => go_home,
    Key::KeyT.down() => enter_typing,
    Key::KeyC.down() => and!(mark_navigating, foreground_chrome, enter_inapp),
    Key::KeyF.down() => and!(mark_navigating, foreground_finder, enter_inapp),
    Key::KeyG.down() => and!(mark_navigating, foreground_ghostty, enter_inapp),
    Key::KeyZ.down() => and!(mark_navigating, foreground_zed, enter_inapp),
    Key::Space.down() => and!(tap_cmd_space, enter_typing),
)]
pub struct NavLayer;

impl NavLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
