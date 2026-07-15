//! The toggle source's handler, and the paused arm's passthrough (with the unpause chord).

use bind::Node;
use freddie_keys::{Key, KeyEvent, PressType};

use crate::state::{Mercury, PausedPath, PowerPath};
use crate::{MercuryEffect, ToggleEvent};

/// A toggle was requested (the menu bar's Toggle): flip paused/unpaused.
///
/// Bound at the root, so it fires whether or not the layer is descended into. It is the
/// menu-driven way to pause or resume, alongside `p` in home and the `cmd`-`alt`-`p` chord.
pub(crate) const fn on_toggle(_ev: &ToggleEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.power.toggle();
    Vec::new()
}

/// Any key while paused: pass it through untouched, except the unpause chord `cmd`-`alt`-`p`.
///
/// The paused arm does not descend into the layer, so this catch-all is what every key reaches.
/// It tracks which command keys are held (there is nowhere better with one-handler-per-event
/// dispatch), and when `p` goes down with `cmd` and `alt` held it unpauses, releasing those
/// modifiers so they are not left stuck, and swallows the `p`. Everything else is re-emitted, so
/// the keyboard is normal while paused.
pub(crate) fn pass_through(ev: &KeyEvent, mut node: Node<PausedPath, ()>) -> Vec<MercuryEffect> {
    let down = ev.press == PressType::Down;
    let paused = node.parent.get_mut();
    match ev.key {
        Key::MetaLeft | Key::MetaRight => paused.held.cmd = down.then_some(ev.key),
        Key::AltLeft | Key::AltRight => paused.held.alt = down.then_some(ev.key),
        _ => {}
    }

    if ev.key == Key::KeyP
        && down
        && let (Some(cmd), Some(alt)) = (paused.held.cmd, paused.held.alt)
    {
        node.parent.ascend_to::<PowerPath>().get_mut().unpause();
        return vec![
            MercuryEffect::Emit(KeyEvent {
                key: cmd,
                press: PressType::Up,
            }),
            MercuryEffect::Emit(KeyEvent {
                key: alt,
                press: PressType::Up,
            }),
        ];
    }

    vec![MercuryEffect::Emit(ev.clone())]
}
