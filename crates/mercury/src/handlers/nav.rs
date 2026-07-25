//! Nav-layer handlers: foreground an app and enter its in-app layer.
//!
//! Picking an app emits the foreground effect and switches straight to the in-app
//! layer, marking a navigation in flight. The app is not recorded here; the watcher
//! reports the app that actually comes up, and [`record_front_app`](super::record_front_app)
//! records it and clears the flag. Until then the in-app level is empty (see
//! [`app_data`](crate::state)), so the old app's bindings do not apply in the gap.

use bind::AscendState;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::{Complete, Completed, HasStop, IntoAncestor, MaybeInvalidated};

use crate::effect::tap;
use crate::state::{AppLayer, Mercury, MercuryPath, TypingLayer};
use crate::{App, MercuryEffect};

/// Foreground `app` and enter the in-app layer, with the navigation marked in flight.
///
/// It takes the root, because its callers are leavers: entering the in-app layer replaces the
/// node they were bound on, so each consumed its path to get here.
fn navigate(root: &mut Mercury, app: App) -> Vec<MercuryEffect> {
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    let mut effects = root.set_layer(inapp);
    effects.push(timer);
    effects.push(MercuryEffect::Foreground(app));
    effects
}

pub(crate) fn open_chrome<'a, E, P>(
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
    let effects = navigate(root, App::Chrome);
    (effects, root.complete())
}
pub(crate) fn open_finder<'a, E, P>(
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
    let effects = navigate(root, App::Finder);
    (effects, root.complete())
}
pub(crate) fn open_ghostty<'a, E, P>(
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
    let effects = navigate(root, App::Ghostty);
    (effects, root.complete())
}
pub(crate) fn open_zed<'a, E, P>(
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
    let effects = navigate(root, App::Zed);
    (effects, root.complete())
}

/// `space` in nav: open Spotlight and land in typing, so what you type next reaches its field.
///
/// Not a [`navigate`]: Spotlight is a text field rather than an app with its own in-app bindings,
/// and it is opened with its own chord rather than by foregrounding anything. The tap comes before
/// the transition, so the modifier downs typing's `open` emits land on Spotlight rather than on the
/// app being left.
pub(crate) fn open_spotlight<'a, E, P>(
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
    let mut effects = vec![tap(Key::Space, ModifierFlags::COMMAND)];
    effects.extend(root.set_layer(TypingLayer::new()));
    (effects, root.complete())
}
