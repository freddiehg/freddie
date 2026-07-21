//! Resize-layer handlers: place the focused window and return home.

use bind::Node;
use freddie_windows::{Frame, WindowFrame};
use laserbeam::Ascend;

use super::and_go_home_from;
use crate::MercuryEffect;
use crate::state::{MercuryPath, Windows};

/// The whole visible frame.
const fn maximized(visible: Frame) -> Frame {
    visible
}

/// The left half, full height.
const fn left_of(visible: Frame) -> Frame {
    Frame {
        width: visible.width / 2.0,
        ..visible
    }
}

/// The right half, full height. Abuts [`left_of`] exactly.
const fn right_of(visible: Frame) -> Frame {
    Frame {
        x: visible.x + visible.width / 2.0,
        width: visible.width / 2.0,
        ..visible
    }
}

pub(crate) fn maximize<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    place(node.parent, maximized)
}

pub(crate) fn left_half<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    place(node.parent, left_of)
}

pub(crate) fn right_half<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    place(node.parent, right_of)
}

/// Put the focused window in the frame `within` picks out of its screen's visible frame,
/// and return home.
///
/// The effects are empty when there is no focused window or no screen has been reported.
/// The layer returns home either way.
fn place<'a, P: Ascend<MercuryPath<'a>>>(
    path: P,
    within: impl Fn(Frame) -> Frame,
) -> Vec<MercuryEffect> {
    let root = path.ascend();
    let effects = target(&root.windows, within)
        .map_or_else(Vec::new, |target| vec![MercuryEffect::SetFrame(target)]);
    and_go_home_from(root, effects)
}

/// The focused window and the frame it is going to.
fn target(windows: &Windows, within: impl Fn(Frame) -> Frame) -> Option<WindowFrame> {
    let focused = windows.focused()?;
    let monitor = windows.monitor_for(focused.frame)?;
    Some(WindowFrame {
        window: focused.window,
        frame: within(monitor.visible),
    })
}

#[cfg(test)]
// The frames here are halves of integers, exactly representable, so the placements are
// exact and comparing them exactly is the point.
#[expect(clippy::float_cmp)]
mod tests {
    use super::{Frame, left_of, maximized, right_of};

    const SCREEN: Frame = Frame {
        x: 0.0,
        y: 25.0,
        width: 1600.0,
        height: 900.0,
    };

    #[test]
    fn maximize_is_the_whole_visible_frame() {
        assert_eq!(maximized(SCREEN), SCREEN);
    }

    #[test]
    fn the_halves_split_the_width_and_keep_the_height() {
        let left = left_of(SCREEN);
        let right = right_of(SCREEN);

        assert_eq!(left.x, SCREEN.x);
        assert_eq!(right.x, SCREEN.x + SCREEN.width / 2.0);
        assert_eq!(left.width, right.width);
        assert_eq!(left.width + right.width, SCREEN.width);
        assert_eq!(left.y, SCREEN.y);
        assert_eq!(right.y, SCREEN.y);
        assert_eq!(left.height, SCREEN.height);
        assert_eq!(right.height, SCREEN.height);
    }

    /// The halves meet exactly, leaving no gap and no overlap.
    #[test]
    fn the_halves_abut() {
        let left = left_of(SCREEN);
        let right = right_of(SCREEN);
        assert_eq!(left.x + left.width, right.x);
    }

    /// An offset screen (a second display, or a dock on the left) is respected.
    #[test]
    fn placements_are_relative_to_the_visible_frame() {
        let offset = Frame {
            x: 1600.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        assert_eq!(left_of(offset).x, 1600.0);
        assert_eq!(right_of(offset).x, 2100.0);
    }
}
