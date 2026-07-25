//! Home-layer handlers: the transitions into the other layers. (`q`'s quit is shared with the
//! menu bar; see [`super::quit`].)
//!
//! Every transition sets the layer through `set_layer` and returns its flush. Most are between
//! command layers, so the flush is empty; entering typing (open) and leaving it (close) are the
//! ones that carry effects.
//!
//! Every one of them ends at the root, and says so by not matching the state at all: the state
//! reaches the root on either branch, and what each completes is the walk it took to get there.
//! On the invalidated branch that re-roots the leave, which is what these mean — wherever the
//! descent below stopped, this dispatch ends at the root, in the layer just set.
//!
//! Each is generic over the event and the path, so any trigger and any node that reaches the
//! root can bind it from its own place in the tree.

use bind::AscendState;
use laserbeam::{Complete, Completed, HasStop, IntoAncestor, MaybeInvalidated};

use super::go_home;
use crate::MercuryEffect;
use crate::state::{AppLayer, MercuryPath, NavLayer, ResizeLayer, SiteLayer, TypingLayer};

/// `escape` anywhere, and a layer's idle-timeout: go back to the home layer.
///
/// Typing has to bind `escape` explicitly, because a plain escape passes through there.
pub(crate) fn to_home<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let effects = go_home(root);
    (effects, root.complete())
}

/// `n`: enter the nav layer. Bound from home and from the in-app layer.
///
/// Nav arms an idle-timeout, so its constructor also hands back the effect that schedules it.
pub(crate) fn to_nav<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let (nav, timer) = NavLayer::new();
    let mut effects = root.set_layer(nav);
    effects.push(timer);
    (effects, root.complete())
}

/// `t`: enter the typing layer. Bound from home, from the in-app layer, and from the app and
/// site levels below it, whose own handlers end there too.
pub(crate) fn to_typing<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let effects = root.set_layer(TypingLayer::new());
    (effects, root.complete())
}

/// `i` in home: enter the in-app layer for whatever app is foregrounded.
pub(crate) fn to_inapp<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let (inapp, timer) = AppLayer::new();
    let mut effects = root.set_layer(inapp);
    effects.push(timer);
    (effects, root.complete())
}

/// `u` in home: enter the per-tab layer.
///
/// Next to `i` under the same finger, because the two are neighbours in meaning as well: `i` is
/// what the frontmost app can do, `u` is what the site in its front tab can do.
pub(crate) fn to_site<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let (site, timer) = SiteLayer::new();
    let mut effects = root.set_layer(site);
    effects.push(timer);
    (effects, root.complete())
}

/// `r` in home: enter the resize layer.
pub(crate) fn to_resize<'a, E, P>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: Complete<P>,
{
    let root: MercuryPath<'a> = st.state.into_ancestor();
    let (resize, timer) = ResizeLayer::new();
    let mut effects = root.set_layer(resize);
    effects.push(timer);
    (effects, root.complete())
}
