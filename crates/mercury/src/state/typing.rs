use bind::Bind;
use freddie_keys::Key;

use crate::MercuryStruct;
#[allow(clippy::wildcard_imports)]
use crate::handlers::*;

use super::LayerPath;

/// The typing layer. It binds only `escape` (`cmd`-`escape` leaves to home); every other key
/// falls to the root, which passes it through because typing is a passthrough layer.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::Escape.down() => maybe_go_home)]
pub struct TypingLayer {}

impl TypingLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {}
    }
}
