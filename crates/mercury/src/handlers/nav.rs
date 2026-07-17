//! Nav-layer handlers: foreground an app and enter its in-app layer.
//!
//! Picking an app emits the foreground effect and switches straight to the in-app
//! layer, marking a navigation in flight. The app is not recorded here; the watcher
//! reports the app that actually comes up, and [`record_front_app`](super::record_front_app)
//! records it and clears the flag. Until then the in-app level is empty (see
//! [`app_data`](crate::state)), so the old app's bindings do not apply in the gap.

use bind::Node;
use freddie_keys::KeyEvent;

use crate::state::{AppLayer, MercuryPath, NavLayerPath};
use crate::{App, MercuryEffect};

/// Foreground `app` and enter the in-app layer, with the navigation marked in flight.
fn navigate(path: NavLayerPath<'_>, app: App) -> Vec<MercuryEffect> {
    // Ascend to the root regardless of the levels between: `foreground` and the layer both live
    // on it.
    let root = path.ascend_to::<MercuryPath>();
    root.foreground.start_navigating();
    let mut effects = root.set_layer(AppLayer {});
    effects.push(MercuryEffect::Foreground(app));
    effects
}

pub(crate) fn open_chrome(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Chrome)
}
pub(crate) fn open_finder(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Finder)
}
pub(crate) fn open_ghostty(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Ghostty)
}
pub(crate) fn open_zed(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Zed)
}
