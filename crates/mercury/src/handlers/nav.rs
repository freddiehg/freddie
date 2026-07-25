//! Nav-layer units: mark a navigation in flight, foreground an app, and the Spotlight chord.
//!
//! Picking an app is one gesture of three units, `and!(mark_navigating, foreground_x,
//! enter_inapp)`: the flag, the effect, and the layer. The app is not recorded here; the watcher
//! reports the app that actually comes up, and [`record_front_app`](super::record_front_app)
//! records it and clears the flag. Until then the in-app level is empty (see
//! [`app_data`](crate::state)), so the old app's bindings do not apply in the gap.

use bind::AscendState;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::{Complete, Completed, HasStop, IntoAncestor, MaybeInvalidated};

use crate::effect::tap;
use crate::state::MercuryPath;
use crate::{App, MercuryEffect};

/// The navigation is in flight: the watcher has not confirmed the new front app yet.
///
/// It writes `foreground`, which lives on the root, so it ends there. Its gesture ends at the
/// root anyway, since `enter_inapp` follows it, so the ending is truthful.
pub(crate) fn mark_navigating<'a, E, P>(
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
    root.foreground.start_navigating();
    (vec![], root.complete())
}

/// One unit per app: the effect and nothing else, so it runs on any state.
macro_rules! foreground_unit {
    ($($handler:ident => $app:ident),* $(,)?) => {$(
        pub(crate) fn $handler<E, P: HasStop + Complete<P>>(
            _ev: &E,
            _snap: (),
            st: AscendState<'_, P>,
        ) -> (Vec<MercuryEffect>, Completed<P>) {
            (vec![MercuryEffect::Foreground(App::$app)], st.complete())
        }
    )*};
}

foreground_unit! {
    foreground_chrome => Chrome,
    foreground_finder => Finder,
    foreground_ghostty => Ghostty,
    foreground_zed => Zed,
}

/// Spotlight's own chord. It is not a [`foreground_unit`]: Spotlight is a text field rather than
/// an app with in-app bindings, and it is opened with a chord rather than by foregrounding
/// anything. The tap comes first in its gesture, so the modifier downs that typing's open emits
/// land on Spotlight rather than on the app being left.
pub(crate) fn tap_cmd_space<E, P: HasStop + Complete<P>>(
    _ev: &E,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![tap(Key::Space, ModifierFlags::COMMAND)], st.complete())
}
