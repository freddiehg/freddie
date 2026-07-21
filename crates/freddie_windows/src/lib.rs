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
//! screen rather than the one the app started on.
//!
//! Requires the Accessibility permission, the same one the keyboard tap needs.
//!
//! macOS only.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::RwLock;

use accessibility_sys::{
    AXIsProcessTrusted, AXUIElementCopyAttributeValue, AXUIElementCreateApplication,
    AXUIElementRef, AXUIElementSetAttributeValue, AXValueCreate, AXValueGetValue, AXValueType,
    kAXFocusedWindowAttribute, kAXPositionAttribute, kAXSizeAttribute, kAXValueTypeCGPoint,
    kAXValueTypeCGSize,
};
use block2::RcBlock;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use core_graphics::geometry::{CGPoint, CGSize};
use objc2_app_kit::{NSApplicationDidChangeScreenParametersNotification, NSScreen, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter};

/// Where a window should end up, as a fraction of the screen's visible frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// The whole visible frame.
    Maximize,
    LeftHalf,
    RightHalf,
}

/// A rectangle in Accessibility coordinates: origin top-left, y increasing down.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
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
pub struct Monitor {
    pub full: Frame,
    pub visible: Frame,
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
    #[expect(unsafe_code)]
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
    tracing::debug!(
        count = monitors.len(),
        ?monitors,
        "monitors, in accessibility coordinates"
    );
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
    #[expect(unsafe_code)]
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

    let monitor = monitor_for(window_frame(window)).ok_or(WindowError::NotInitialized)?;
    let target = placement.within(monitor.visible);
    set_frame(window, target);

    // SAFETY: `focused_window` returned a +1 reference; this balances it.
    #[expect(unsafe_code)]
    unsafe {
        CFRelease(window.cast());
    }
    tracing::debug!(?placement, ?target, "placed the focused window");
    Ok(())
}

/// The monitor a window is on: the one whose full frame contains the window's top-left
/// corner, or the first monitor if none does or the frame could not be read. `None` only
/// if [`init`] never cached any monitor.
fn monitor_for(frame: Option<Frame>) -> Option<Monitor> {
    let monitors = MONITORS.read().ok()?.clone();
    let first = *monitors.first()?;
    let chosen = frame
        .and_then(|f| monitors.iter().find(|m| m.full.contains(f.x, f.y)).copied())
        .unwrap_or(first);
    Some(chosen)
}

/// A +1 CoreFoundation reference, released when it drops.
///
/// CF's rule is that a function with `Create` or `Copy` in its name hands you ownership,
/// so `AXUIElementCopyAttributeValue`, `AXUIElementCreateApplication`, and `AXValueCreate`
/// all return one of these. Wrapping it is what makes the release impossible to forget
/// when a `?` or an early return is added between the call and the end of the function.
///
/// Deliberately not `Copy` and not `Clone`: two of these naming one reference would
/// release it twice.
struct Owned(CFTypeRef);

impl Owned {
    /// Take ownership of what a `Create` or `Copy` returned, or `None` if it returned
    /// nothing.
    fn new(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: an `Owned` is only built from a +1 reference, and only here is it
        // released, once.
        #[expect(unsafe_code)]
        unsafe {
            CFRelease(self.0);
        }
    }
}

/// One `AXValue` attribute: the name it is read by, the `AXValueType` it holds, and the
/// Rust type that type means.
///
/// All three together, because `AXValueGetValue` writes through an untyped pointer: an
/// attribute read with the wrong kind, or into the wrong type, is a mismatch nothing would
/// otherwise catch.
trait AxAttribute {
    const NAME: &'static str;
    const KIND: AXValueType;
    type Value: Copy + Default;
}

struct Position;
impl AxAttribute for Position {
    const NAME: &'static str = kAXPositionAttribute;
    const KIND: AXValueType = kAXValueTypeCGPoint;
    type Value = CGPoint;
}

struct Size;
impl AxAttribute for Size {
    const NAME: &'static str = kAXSizeAttribute;
    const KIND: AXValueType = kAXValueTypeCGSize;
    type Value = CGSize;
}

/// The value of one attribute of `element`, owned.
fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<Owned> {
    let attribute = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is live and `attribute` a live string. On success the
    // out-parameter receives a +1 reference; on failure it is untouched.
    #[expect(unsafe_code)]
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    (status == 0).then(|| Owned::new(value))?
}

/// Read one `AXValue` attribute of `element`.
fn ax_value<A: AxAttribute>(element: AXUIElementRef) -> Option<A::Value> {
    let value = copy_attribute(element, A::NAME)?;
    let mut out = A::Value::default();
    // SAFETY: `value` is a live `AXValue`, and the impl pairs `A::KIND` with `A::Value`,
    // so a successful read writes an `A::Value` into an `A::Value`.
    #[expect(unsafe_code)]
    let got = unsafe {
        AXValueGetValue(
            value.0.cast_mut().cast(),
            A::KIND,
            std::ptr::from_mut(&mut out).cast(),
        )
    };
    if !got {
        // The attribute did not hold the type it is documented to hold, which is the app's
        // Accessibility implementation misbehaving. Logged rather than fatal: a daemon that
        // remaps the keyboard should not die because some app answered oddly.
        tracing::warn!(
            attribute = A::NAME,
            "an AXValue was not the type it should be"
        );
    }
    got.then_some(out)
}

/// A window's frame, in Accessibility coordinates, or `None` if either half of it
/// cannot be read.
fn window_frame(window: AXUIElementRef) -> Option<Frame> {
    let origin = ax_value::<Position>(window)?;
    let size = ax_value::<Size>(window)?;
    Some(Frame {
        x: origin.x,
        y: origin.y,
        width: size.width,
        height: size.height,
    })
}

/// The focused window of the frontmost app, as a +1 reference the caller releases.
fn focused_window() -> Option<AXUIElementRef> {
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();

    // SAFETY: `pid` names a live process, and `AXUIElementCreateApplication` takes
    // no ownership of it. The returned element is +1 and released below.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid) };

    let attribute = CFString::new(kAXFocusedWindowAttribute);
    let mut window: *const c_void = std::ptr::null();
    // SAFETY: `app` is a live element and `attribute` a live string. On success the
    // out-parameter receives a +1 reference; on failure it is untouched.
    #[expect(unsafe_code)]
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
        #[expect(unsafe_code)]
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
#[expect(clippy::float_cmp)]
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
