use bind::Bind;

use crate::MercuryStruct;

use super::LayerPath;

/// The typing layer. It binds nothing: every key falls to the root, which runs it through the `jk`
/// sequence and passes it through, because typing is a passthrough layer. `jk` is the way out.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
pub struct TypingLayer {}

impl TypingLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}
