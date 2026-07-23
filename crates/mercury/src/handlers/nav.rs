//! Nav-layer handlers: foreground an app and enter its in-app layer.
//!
//! Picking an app emits the foreground effect and switches straight to the in-app
//! layer, marking a navigation in flight. The app is not recorded here; the watcher
//! reports the app that actually comes up, and [`record_front_app`](super::record_front_app)
//! records it and clears the flag. Until then the in-app level is empty (see
//! [`app_data`](crate::state)), so the old app's bindings do not apply in the gap.

use bind::Node;
use freddie_keys::{Key, ModifierFlags};
use laserbeam::Ascend;

use super::to_typing;
use crate::effect::tap;
use crate::state::{AppLayer, MercuryPath};
use crate::{App, MercuryEffect};

/// Foreground `app` and enter the in-app layer, with the navigation marked in flight.
///
/// Generic over the path, so every opener binds it from its own node.
fn navigate<'a, P: Ascend<MercuryPath<'a>>>(path: P, app: App) -> Vec<MercuryEffect> {
    let root: MercuryPath<'_> = path.ascend_mut();
    root.foreground.start_navigating();
    let (inapp, timer) = AppLayer::new();
    let mut effects = root.set_layer(inapp);
    effects.push(timer);
    effects.push(MercuryEffect::Foreground(app));
    effects
}

pub(crate) fn open_chrome<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Chrome)
}
pub(crate) fn open_finder<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Finder)
}
pub(crate) fn open_ghostty<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Ghostty)
}
pub(crate) fn open_zed<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Zed)
}

/// `space` in nav: open Spotlight and land in typing, so what you type next reaches its field.
///
/// Not a [`navigate`]: Spotlight is a text field rather than an app with its own in-app bindings,
/// and it is opened with its own chord rather than by foregrounding anything. The tap comes before
/// the transition, so the modifier downs typing's `open` emits land on Spotlight rather than on the
/// app being left.
pub(crate) fn open_spotlight<'a, E, P: Ascend<MercuryPath<'a>>>(
    ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let mut effects = vec![tap(Key::Space, ModifierFlags::COMMAND)];
    effects.extend(to_typing(ev, node));
    effects
}
