//! Placing the focused window of the frontmost app on screen.
//!
//! [`place`] moves and resizes the focused window through the Accessibility API,
//! which is the only way to set a window's frame: `CGWindow` can read geometry but
//! not write it. It happens immediately, with no animation, in single-digit to low
//! tens of milliseconds.
//!
//! [`init`] must be called once, on the main thread, before [`place`]. It reads
//! every monitor's frame, which is `AppKit` and therefore main-thread-bound, caches
//! them, and registers an observer that re-reads them whenever the screen
//! arrangement changes (a monitor plugged, unplugged, or rearranged). [`place`] runs
//! off the main thread against that cache: it finds the monitor the focused window
//! is on and places the window within that monitor's visible frame. So a window on a
//! second display, or on a display connected after launch, still fills its own
//! screen rather than the one mercury started on.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::RwLock;

use accessibility_sys::{
    AXIsProcessTrusted, AXUIElementCopyAttributeValue, AXUIElementCreateApplication,
    AXUIElementRef, AXUIElementSetAttributeValue, AXValueCreate, AXValueGetValue,
    kAXFocusedWindowAttribute, kAXPositionAttribute, kAXSizeAttribute, kAXValueTypeCGPoint,
    kAXValueTypeCGSize,
};
use block2::RcBlock;
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::CFString;
use core_graphics::geometry::{CGPoint, CGSize};
use objc2_app_kit::{NSApplicationDidChangeScreenParametersNotification, NSScreen, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter};

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

impl Frame {
    /// Whether `(x, y)` lies in this frame.
    const fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// A monitor: its full frame, for locating a window, and its visible frame, the area
/// a placement fills (the full frame minus the menu bar and the dock). Both in
/// Accessibility coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Monitor {
    full: Frame,
    visible: Frame,
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

/// Every monitor's frames, in Accessibility coordinates. Read on the main thread by
/// [`init`] and refreshed there by the screen-change observer; read off the main
/// thread by [`place`].
static MONITORS: RwLock<Vec<Monitor>> = RwLock::new(Vec::new());

/// Reads every monitor's frames and caches them.
///
/// Also registers an observer that re-reads them when the screen arrangement
/// changes, so [`place`] can run off the main thread against a cache that never goes
/// stale. Must be called on the main thread: `NSScreen` is `AppKit`.
///
/// # Errors
///
/// [`WindowError::NotMainThread`] if called elsewhere, [`WindowError::NotTrusted`]
/// if Accessibility has not been granted, and [`WindowError::NoScreen`] if there is
/// no monitor at all.
pub fn init() -> Result<(), WindowError> {
    let mtm = MainThreadMarker::new().ok_or(WindowError::NotMainThread)?;

    // SAFETY: a plain C predicate over process state; takes no arguments.
    #[allow(unsafe_code)]
    if !unsafe { AXIsProcessTrusted() } {
        return Err(WindowError::NotTrusted);
    }

    let monitors = read_monitors(mtm);
    if monitors.is_empty() {
        return Err(WindowError::NoScreen);
    }
    store_monitors(monitors);
    register_screen_change_observer();
    Ok(())
}

/// Reads every monitor's full and visible frame, in Accessibility coordinates.
///
/// `NSScreen` has a global bottom-left origin and Accessibility a global top-left
/// one, so the y flips around the PRIMARY display's height, not each screen's own.
/// That is what places a monitor above or beside the primary at the right global y.
fn read_monitors(mtm: MainThreadMarker) -> Vec<Monitor> {
    let screens = NSScreen::screens(mtm);

    // The primary display sits at the global origin; its full height is the flip axis.
    let primary_height = screens
        .iter()
        .find(|s| {
            let o = s.frame().origin;
            o.x == 0.0 && o.y == 0.0
        })
        .or_else(|| screens.iter().next())
        .map_or(0.0, |s| s.frame().size.height);

    let to_ax = |rect: objc2_foundation::NSRect| Frame {
        x: rect.origin.x,
        y: primary_height - (rect.origin.y + rect.size.height),
        width: rect.size.width,
        height: rect.size.height,
    };

    screens
        .iter()
        .map(|screen| Monitor {
            full: to_ax(screen.frame()),
            visible: to_ax(screen.visibleFrame()),
        })
        .collect()
}

/// Replaces the cached monitors.
fn store_monitors(monitors: Vec<Monitor>) {
    tracing::debug!(count = monitors.len(), ?monitors, "monitors, in accessibility coordinates");
    if let Ok(mut guard) = MONITORS.write() {
        *guard = monitors;
    }
}

/// Registers an observer that re-reads the monitors when the screen arrangement
/// changes.
///
/// Leaked on purpose: the observation lasts the whole process, and deregistering
/// would only matter at shutdown, when the process is going away regardless.
fn register_screen_change_observer() {
    let block = RcBlock::new(|_notif: NonNull<NSNotification>| {
        // Delivered on the main thread, so reading `NSScreen` here is sound.
        if let Some(mtm) = MainThreadMarker::new() {
            store_monitors(read_monitors(mtm));
            tracing::debug!("re-read monitors after a screen-arrangement change");
        }
    });

    // SAFETY: `NSApplicationDidChangeScreenParametersNotification` is an immutable
    // extern static. The block captures nothing and is invoked on the main thread.
    // The token and block are leaked so the observation lasts the process.
    #[allow(unsafe_code)]
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(token);
    std::mem::forget(block);
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
    let window = focused_window().ok_or(WindowError::NoFocusedWindow)?;

    let monitor = monitor_for(window).ok_or(WindowError::NotInitialized)?;
    let target = placement.within(monitor.visible);
    set_frame(window, target);

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[allow(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    tracing::debug!(?placement, ?target, "placed the focused window");
    Ok(())
}

/// The monitor the focused window is on: the one whose full frame contains the
/// window's top-left corner, or the first monitor if none does or the position could
/// not be read. `None` only if [`init`] never cached any monitor.
fn monitor_for(window: AXUIElementRef) -> Option<Monitor> {
    let monitors = MONITORS.read().ok()?.clone();
    let first = *monitors.first()?;
    let chosen = window_origin(window)
        .and_then(|p| monitors.iter().find(|m| m.full.contains(p.x, p.y)).copied())
        .unwrap_or(first);
    Some(chosen)
}

/// The focused window's top-left corner, in Accessibility coordinates, or `None` if
/// it cannot be read.
fn window_origin(window: AXUIElementRef) -> Option<CGPoint> {
    let attribute = CFString::new(kAXPositionAttribute);
    let mut value: *const c_void = std::ptr::null();
    // SAFETY: `window` is a live element and `attribute` a live string. On success
    // the out-parameter receives a +1 `AXValue`; on failure it is untouched.
    #[allow(unsafe_code)]
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            window,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }

    let mut point = CGPoint::new(0.0, 0.0);
    // SAFETY: `value` is a +1 `AXValue` of CGPoint type; `AXValueGetValue` copies it
    // into `point`. The value is released afterward.
    #[allow(unsafe_code)]
    let got = unsafe {
        let ok = AXValueGetValue(
            value.cast_mut().cast(),
            kAXValueTypeCGPoint,
            std::ptr::from_mut(&mut point).cast(),
        );
        CFRelease(value);
        ok
    };
    got.then_some(point)
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

    #[test]
    fn contains_is_half_open() {
        let f = Frame {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        assert!(f.contains(0.0, 0.0));
        assert!(f.contains(99.0, 49.0));
        assert!(!f.contains(100.0, 0.0), "right edge is excluded");
        assert!(!f.contains(0.0, 50.0), "bottom edge is excluded");
        assert!(!f.contains(-1.0, 0.0));
    }

    /// A window's corner picks the monitor it sits on, which is how [`monitor_for`]
    /// chooses the screen to place within. Two monitors side by side, the second
    /// shorter, the way an external display next to a laptop is.
    #[test]
    fn a_point_picks_the_monitor_it_is_on() {
        let left = Frame {
            x: 0.0,
            y: 0.0,
            width: 1600.0,
            height: 900.0,
        };
        let right = Frame {
            x: 1600.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        let monitors = [left, right];
        let pick = |x, y| monitors.iter().position(|m| m.contains(x, y));
        assert_eq!(pick(10.0, 10.0), Some(0));
        assert_eq!(pick(1700.0, 10.0), Some(1));
        assert_eq!(pick(3000.0, 10.0), None, "off both monitors");
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
