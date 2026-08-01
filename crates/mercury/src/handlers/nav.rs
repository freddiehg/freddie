//! Nav-layer handlers: the app chooser and the Spotlight chord.
//!
//! A choice marks the navigation in flight, asks for the app, and enters the in-app layer. The
//! app is not recorded here; the watcher reports the app that actually comes up, and
//! [`record_front_app`](super::record_front_app) records it and clears the flag. Until then the
//! in-app level is empty (see [`app_data`](crate::state)), so the old app's bindings do not
//! apply in the gap.

use freddie_keys::{Key, ModifierFlags};
use laserbeam::{Completed, CompletesTo, HasStop, IntoAncestor};

use crate::effect::tap;
use crate::state::{AndReturnHome, AppLayer, MercuryPath};
use crate::{App, MercuryEffect};

/// The navigation is in flight: the watcher has not confirmed the new front app yet.
///
/// It writes `foreground`, which lives on the root, so it ends there. Its gesture ends at the
/// root anyway, since `enter_inapp` follows it, so the ending is truthful.
/// A nav choice: mark the navigation in flight, ask for `app`, and enter the in-app layer.
pub(crate) fn open<'a, E, P>(app: App) -> impl Fn(&E, (), P) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    move |_ev, _snap, p| {
        let root: MercuryPath<'a> = p.into_ancestor();
        root.foreground = None;
        let mut effects = vec![MercuryEffect::Foreground(app)];
        let (wrapped, timer) = AndReturnHome::new(AppLayer::new());
        effects.extend(root.set_layer(wrapped));
        effects.push(timer);
        (effects, root.complete())
    }
}

/// Spotlight's own chord. It is not a [`foreground_unit`]: Spotlight is a text field rather than
/// an app with in-app bindings, and it is opened with a chord rather than by foregrounding
/// anything. The tap comes first in its gesture, so the modifier downs that typing's open emits
/// land on Spotlight rather than on the app being left.
pub(crate) fn tap_cmd_space<E, P: HasStop + CompletesTo<P>>(
    _ev: &E,
    _snap: (),
    p: P,
) -> (Vec<MercuryEffect>, Completed<P>) {
    (vec![tap(Key::Space, ModifierFlags::COMMAND)], p.complete())
}
