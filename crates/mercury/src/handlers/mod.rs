//! The key and foreground handlers, one module per layer.
//!
//! Each is the one scheduled shape,
//! `fn(&SourceEvent, Snap, AscendState<P>) -> (Vec<MercuryEffect>, Completed<P>)`: the event, what
//! its pre snapped before the descent, and what the descent left of the path it is bound on. It
//! returns inert effects and the leave it completed to. `crate::state` glob-imports this module so
//! the derive-generated dispatch can name them.
//!
//! Which of the two arms a handler takes is what it means. A stayer completes where it stands and
//! reads the tree through `HasAncestor::ancestor`; a leaver consumes the path with
//! `IntoAncestor::into_ancestor`, does its work at the root, and completes there, which is what
//! tells everything above it that the layer it was bound on is gone. The `Invalidated` arm is the
//! same answer either way: something below already left, so there is no path to act through, and
//! the leave is forwarded untouched.

mod app;
mod foreground;
mod home;
mod nav;
mod overlay;
mod quit;
mod resize;
mod root;
mod tab;
mod window;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use overlay::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use root::*;
pub(crate) use tab::*;
pub(crate) use window::*;

use crate::MercuryEffect;
use crate::state::{HomeLayer, Mercury};

/// Go to the home layer, returning the modifier flush (empty unless leaving a passthrough layer).
/// The one place the home layer is entered.
pub(crate) fn go_home(root: &mut Mercury) -> Vec<MercuryEffect> {
    root.set_layer(HomeLayer::new())
}

/// Ask for `effects`, then return home.
///
/// A layer stays only if its actions make sense to do repeatedly. Walking tmux's panes and
/// refreshing Chrome do, so the in-app layers stay. Placing a window does not: repeating it is
/// a no-op, and anything else is a different choice. So resize is a one-shot chooser, and this
/// is how it leaves. (Nav also leaves after one choice, but into the in-app layer rather than
/// home; see [`super::nav`].)
///
/// It takes the root, because its callers are leavers and already hold it: a leaver consumed its
/// path to get here, and completes at the root once this hands the effects back.
pub(crate) fn and_go_home_from(
    root: &mut Mercury,
    effects: impl IntoIterator<Item = MercuryEffect>,
) -> Vec<MercuryEffect> {
    let mut effects: Vec<_> = effects.into_iter().collect();
    effects.extend(go_home(root));
    effects
}
