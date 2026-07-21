use bind::Bind;

use crate::MercuryStruct;

use super::LayerPath;

/// The typing layer. It binds nothing: every key falls to the root, which runs it through the `jk`
/// sequence and passes it through, because typing is a passthrough layer. `jk` is the way out.
/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/typing.txt");

#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
pub struct TypingLayer;

impl TypingLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}
