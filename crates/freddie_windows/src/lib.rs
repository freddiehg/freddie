//! Placing the focused window of the frontmost app on screen.
//!
//! [`place`] moves and resizes the focused window through the Accessibility API,
//! which is the only way to set a window's frame: `CGWindow` can read geometry but
//! not write it. It happens immediately, with no animation, in single-digit to low
//! tens of milliseconds.
//!
//! [`init`] must be called once, on the main thread, before [`place`]. It reads
//! the screen's visible frame, which is `AppKit` and therefore main-thread-bound,
//! and caches it. [`place`] itself may be called from any thread.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::ffi::c_void;
use std::sync::OnceLock;

use accessibility_sys::{
    AXIsProcessTrusted, AXUIElementCopyAttributeValue, AXUIElementCreateApplication,
    AXUIElementRef, AXUIElementSetAttributeValue, AXValueCreate, kAXFocusedWindowAttribute,
    kAXPositionAttribute, kAXSizeAttribute, kAXValueTypeCGPoint, kAXValueTypeCGSize,
};
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::CFString;
use core_graphics::geometry::{CGPoint, CGSize};
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::MainThreadMarker;

/// Where a window should end up, as a fraction of the screen's visible frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// The whole visible frame.
    Maximize,
    /// The left half.
    LeftHalf,
    /// The right half.
    RightHalf,
}

/// A rectangle in Accessibility coordinates: origin top-left, y increasing down.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Placement {
    /// The frame this placement occupies within `visible`.
    const fn within(self, visible: Frame) -> Frame {
        let half = visible.width / 2.0;
        match self {
            Self::Maximize => visible,
            Self::LeftHalf => Frame {
                width: half,
                ..visible
            },
            Self::RightHalf => Frame {
                x: visible.x + half,
                width: half,
                ..visible
            },
        }
    }
}

/// Placing a window failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowError {
    /// [`init`] was not called, or it failed.
    NotInitialized,
    /// [`init`] was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// There is no screen to place a window on.
    NoScreen,
    /// Nothing is frontmost, or the frontmost app has no focused window.
    NoFocusedWindow,
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotInitialized => "freddie_windows::init was not called",
            Self::NotMainThread => "freddie_windows::init must run on the main thread",
            Self::NotTrusted => "Accessibility is not granted",
            Self::NoScreen => "no screen",
            Self::NoFocusedWindow => "no focused window",
        })
    }
}

impl std::error::Error for WindowError {}

/// The screen's visible frame, read once on the main thread.
static VISIBLE_FRAME: OnceLock<Frame> = OnceLock::new();

/// Read the screen's visible frame and cache it, so [`place`] can run off the main
/// thread.
///
/// Must be called on the main thread: `NSScreen` is `AppKit`. The visible frame
/// excludes the menu bar and the dock, so a maximized window does not hide behind
/// either.
///
/// # Errors
///
/// [`WindowError::NotMainThread`] if called elsewhere, [`WindowError::NotTrusted`]
/// if Accessibility has not been granted, and [`WindowError::NoScreen`] if there
/// is no main screen.
pub fn init() -> Result<(), WindowError> {
    let mtm = MainThreadMarker::new().ok_or(WindowError::NotMainThread)?;

    // SAFETY: a plain C predicate over process state; takes no arguments.
    #[allow(unsafe_code)]
    if !unsafe { AXIsProcessTrusted() } {
        return Err(WindowError::NotTrusted);
    }

    let screen = NSScreen::mainScreen(mtm).ok_or(WindowError::NoScreen)?;
    let full = screen.frame();
    let visible = screen.visibleFrame();

    // `NSScreen` has a bottom-left origin and Accessibility has a top-left one, so
    // the y flips around the screen's height.
    let frame = Frame {
        x: visible.origin.x,
        y: full.size.height - (visible.origin.y + visible.size.height),
        width: visible.size.width,
        height: visible.size.height,
    };
    tracing::debug!(?frame, "screen visible frame, in accessibility coordinates");
    let _ = VISIBLE_FRAME.set(frame);
    Ok(())
}

/// Move and resize the focused window of the frontmost app.
///
/// Immediate, with no animation. Costs single-digit to low tens of milliseconds,
/// so a caller on a latency-sensitive loop should hand this to another thread.
///
/// # Errors
///
/// [`WindowError::NotInitialized`] if [`init`] has not run, and
/// [`WindowError::NoFocusedWindow`] if nothing is frontmost or the frontmost app
/// has no focused window.
pub fn place(placement: Placement) -> Result<(), WindowError> {
    let visible = *VISIBLE_FRAME.get().ok_or(WindowError::NotInitialized)?;
    let target = placement.within(visible);

    let window = focused_window().ok_or(WindowError::NoFocusedWindow)?;
    set_frame(window, target);

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[allow(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    tracing::debug!(?placement, ?target, "placed the focused window");
    Ok(())
}

/// The focused window of the frontmost app, as a +1 reference the caller releases.
fn focused_window() -> Option<AXUIElementRef> {
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();

    // SAFETY: `pid` names a live process, and `AXUIElementCreateApplication` takes
    // no ownership of it. The returned element is +1 and released below.
    #[allow(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid) };

    let attribute = CFString::new(kAXFocusedWindowAttribute);
    let mut window: *const c_void = std::ptr::null();
    // SAFETY: `app` is a live element and `attribute` a live string. On success the
    // out-parameter receives a +1 reference; on failure it is untouched.
    #[allow(unsafe_code)]
    let status = unsafe {
        let s = AXUIElementCopyAttributeValue(
            app,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut window).cast(),
        );
        CFRelease(app.cast());
        s
    };

    (status == 0 && !window.is_null()).then(|| window.cast_mut().cast())
}

/// Set the window's position and size, twice.
///
/// Twice because some apps clamp a move against their current size, so the first
/// position lands short of where it was asked to go and the second lands true.
fn set_frame(window: AXUIElementRef, frame: Frame) {
    let origin = CGPoint::new(frame.x, frame.y);
    let size = CGSize::new(frame.width, frame.height);

    for _ in 0..2 {
        // SAFETY: `AXValueCreate` copies out of the pointer it is given, and both
        // values live for the call. Each returned value is +1 and released here.
        // `window` is a live element; setting an attribute takes no ownership.
        #[allow(unsafe_code)]
        unsafe {
            let position = AXValueCreate(kAXValueTypeCGPoint, (&raw const origin).cast());
            let extent = AXValueCreate(kAXValueTypeCGSize, (&raw const size).cast());
            AXUIElementSetAttributeValue(
                window,
                CFString::new(kAXPositionAttribute).as_concrete_TypeRef(),
                position.cast(),
            );
            AXUIElementSetAttributeValue(
                window,
                CFString::new(kAXSizeAttribute).as_concrete_TypeRef(),
                extent.cast(),
            );
            CFRelease(position.cast());
            CFRelease(extent.cast());
        }
    }
}

#[cfg(test)]
// The frames here are halves of integers, exactly representable, so the
// placements are exact and comparing them exactly is the point.
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Frame, Placement};

    const SCREEN: Frame = Frame {
        x: 0.0,
        y: 25.0,
        width: 1600.0,
        height: 900.0,
    };

    #[test]
    fn maximize_is_the_whole_visible_frame() {
        assert_eq!(Placement::Maximize.within(SCREEN), SCREEN);
    }

    #[test]
    fn the_halves_split_the_width_and_keep_the_height() {
        let left = Placement::LeftHalf.within(SCREEN);
        let right = Placement::RightHalf.within(SCREEN);

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
        let left = Placement::LeftHalf.within(SCREEN);
        let right = Placement::RightHalf.within(SCREEN);
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
        assert_eq!(Placement::LeftHalf.within(offset).x, 1600.0);
        assert_eq!(Placement::RightHalf.within(offset).x, 2100.0);
    }
}
