//! Nav-layer handlers: foreground an app and return home.

use bind::Node;
use freddie_keys::KeyEvent;

use super::and_go_home;
use crate::state::NavLayerPath;
use crate::{App, MercuryEffect};

pub(crate) fn open_chrome(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Chrome)])
}
pub(crate) fn open_ghostty(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Ghostty)])
}
pub(crate) fn open_zed(_ev: &KeyEvent, node: Node<NavLayerPath, ()>) -> Vec<MercuryEffect> {
    and_go_home(node.parent, vec![MercuryEffect::Foreground(App::Zed)])
}
