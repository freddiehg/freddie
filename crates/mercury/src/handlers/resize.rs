//! Resize-layer units: place the focused window.
//!
//! Placing is the whole of each unit; returning home is `go_home` composing after it at the bind
//! site, because placing a window is a choice rather than something you repeat. Each writes
//! `windows`, which lives on the root, so each ends there, and the `go_home` after it ends there
//! too: two root-enders compose, since the state-level `into_ancestor` is total on both branches.

use freddie_windows::{Frame, WindowFrame};
use laserbeam::{Completed, CompletesTo, HasStop, IntoAncestor};

use crate::MercuryEffect;
use crate::state::{HomeLayer, Mercury, MercuryPath, Windows};

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

pub(crate) fn maximize<'a, E, P>(_ev: &E, _snap: (), p: P) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    let root: MercuryPath<'a> = p.into_ancestor();
    let mut effects = place(root, maximized);
    effects.extend(root.set_layer(HomeLayer::new()));
    (effects, root.complete())
}

pub(crate) fn left_half<'a, E, P>(_ev: &E, _snap: (), p: P) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    let root: MercuryPath<'a> = p.into_ancestor();
    let mut effects = place(root, left_of);
    effects.extend(root.set_layer(HomeLayer::new()));
    (effects, root.complete())
}

pub(crate) fn right_half<'a, E, P>(_ev: &E, _snap: (), p: P) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    let root: MercuryPath<'a> = p.into_ancestor();
    let mut effects = place(root, right_of);
    effects.extend(root.set_layer(HomeLayer::new()));
    (effects, root.complete())
}

/// Put the focused window in the frame `within` picks out of its screen's visible frame.
///
/// The effects are empty when there is no focused window or no screen has been reported.
fn place(root: &mut Mercury, within: impl Fn(Frame) -> Frame) -> Vec<MercuryEffect> {
    target(&root.windows, within).map_or_else(Vec::new, |target| root.windows.placing(target))
}

/// Put the focused window back where it was before it was placed, and return home.
pub(crate) fn restore<'a, E, P>(_ev: &E, _snap: (), p: P) -> (Vec<MercuryEffect>, Completed<P>)
where
    P: HasStop + IntoAncestor<MercuryPath<'a>>,
    MercuryPath<'a>: CompletesTo<P>,
{
    let root: MercuryPath<'a> = p.into_ancestor();
    let mut effects = root.windows.restoring();
    effects.extend(root.set_layer(HomeLayer::new()));
    (effects, root.complete())
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
