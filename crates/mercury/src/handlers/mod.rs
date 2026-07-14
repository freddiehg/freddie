//! The key and foreground handlers, one module per layer.
//!
//! Each is a `fn(&SourceEvent, Node<OwnPath, ()>) -> Vec<MercuryEffect>`. It reaches the tree
//! through the path the node carries, and returns inert effects. `crate::tree` glob-imports
//! this module so the derive-generated dispatch can name them.

mod app;
mod foreground;
mod home;
mod nav;
mod resize;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use resize::*;
pub(crate) use typing::*;

use laserbeam::Ascend;

use crate::tree::{HomeLayer, Layer, LayerPath};
use crate::MercuryEffect;

/// Put the layer back in home. The one place the home layer is entered.
pub(crate) fn go_home(layer: &mut LayerPath<'_>) {
    *layer.get_mut() = Layer::Home(HomeLayer {});
}

/// Ask for `effects` and return home.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's panes and
/// refreshing Chrome do, so the in-app layers stay. Choosing an app or a window placement does
/// not: repeating it is a no-op, and anything else is a different choice. So nav and resize are
/// one-shot choosers, and this is how they leave.
///
/// Generic over the path, so every chooser binds it from its own node.
///
/// The layer change is immediate; the effect is not. Foregrounding an app records it only
/// later, when the watcher reports what actually came up, so a following `i` may briefly
/// resolve the in-app layer against the old app. [`on_foregrounded`] retargets it when the
/// real event lands.
pub(crate) fn and_go_home<'a, P: Ascend<LayerPath<'a>>>(
    path: P,
    effects: Vec<MercuryEffect>,
) -> Vec<MercuryEffect> {
    go_home(&mut path.ascend());
    effects
}
