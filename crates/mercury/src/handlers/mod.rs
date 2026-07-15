//! The key and foreground handlers, one module per layer.
//!
//! Each is a `fn(&SourceEvent, Node<OwnPath, ()>) -> Vec<MercuryEffect>`. It reaches the tree
//! through the path the node carries, and returns inert effects. `crate::state` glob-imports
//! this module so the derive-generated dispatch can name them.

mod app;
mod foreground;
mod home;
mod nav;
mod quit;
mod resize;
mod toggle;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use toggle::*;
pub(crate) use typing::*;

use laserbeam::Ascend;

use crate::state::{HomeLayer, Layer, LayerPath};
use crate::MercuryEffect;

/// Put the layer back in home. The one place the home layer is entered.
pub(crate) fn go_home(layer: &mut LayerPath<'_>) {
    *layer.get_mut() = Layer::Home(HomeLayer {});
}

/// Ask for `effects` and return home.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's panes and
/// refreshing Chrome do, so the in-app layers stay. Placing a window does not: repeating it is
/// a no-op, and anything else is a different choice. So resize is a one-shot chooser, and this
/// is how it leaves. (Nav also leaves after one choice, but into the in-app layer rather than
/// home; see [`super::nav`].)
///
/// Generic over the path, so every chooser binds it from its own node.
pub(crate) fn and_go_home<'a, P: Ascend<LayerPath<'a>>>(
    path: P,
    effects: Vec<MercuryEffect>,
) -> Vec<MercuryEffect> {
    go_home(&mut path.ascend());
    effects
}
