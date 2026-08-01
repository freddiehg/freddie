use bind::{Bind, and, if_not_invalidated};
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{App, MercuryStruct};

use super::ReturnHomeLayersPath;

/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/nav.txt");

#[derive(Bind, Debug)]
#[node(parent_path = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::Escape.down() => if_not_invalidated(go_home),
    Key::KeyT.down() => if_not_invalidated(enter_typing),
    Key::KeyC.down() => if_not_invalidated(open(App::Chrome)),
    Key::KeyF.down() => if_not_invalidated(open(App::Finder)),
    Key::KeyG.down() => if_not_invalidated(open(App::Ghostty)),
    Key::KeyZ.down() => if_not_invalidated(open(App::Zed)),
    Key::Space.down() => if_not_invalidated(and!(tap_cmd_space, enter_typing)),
)]
pub struct NavLayer;

impl NavLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
