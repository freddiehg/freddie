//! The windows watcher's reported vocabulary: the pure data its events and snapshots carry.

/// A running app, by process id. `pid_t` is an `i32`, and an `i32` is not a process.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pid(pub i32);

/// A window's `CGWindowID`: the identity that outlives any one `AXUIElement` naming it.
///
/// Elements are created per call, so two for the same window are different pointers and
/// the element itself cannot be the key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WindowId(pub u32);

/// A rectangle in Accessibility coordinates: origin top-left, y increasing down.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    /// Whether `(x, y)` lies in this frame. Half-open: the left and top edges are in, the
    /// right and bottom are not, so abutting frames do not both claim a point.
    #[must_use]
    pub const fn contains(self, x: f64, y: f64) -> bool {
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

/// Placing a window failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowError {
    /// `watch` was called off the main thread.
    NotMainThread,
    /// The Accessibility permission has not been granted.
    NotTrusted,
    /// The watcher has been dropped, so nothing is being observed at all.
    NotWatching,
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMainThread => "freddie_windows::watch must run on the main thread",
            Self::NotTrusted => "Accessibility is not granted",
            Self::NotWatching => "not watching windows",
        })
    }
}

impl std::error::Error for WindowError {}

/// What the windows are doing. One variant per fact the watcher can report; the values a fact
/// invalidates are the consumer's reads to make.
#[derive(Clone, PartialEq, Debug)]
pub enum WindowChange {
    /// A window appeared. Its frame is the consumer's read to make.
    Opened(WindowId),
    /// A window moved: its old frame is dead, and the new one is the consumer's read to make.
    Moved(WindowId),
    /// A window was resized: same contract as [`Moved`](Self::Moved).
    Resized(WindowId),
    /// A window went away. Final: a read landing after this names a window the consumer
    /// already removed.
    Closed(WindowId),
    /// Focus changed in the app with this pid — a notification's report and an activation's
    /// alike, ungated. Which window focus landed on is the consumer's read to make.
    FocusChanged(Pid),
    /// The app and every entry keyed by its pid are gone. Reported after the per-window
    /// [`Closed`](Self::Closed) reports.
    AppGone(Pid),
    /// The monitors changed, with the new arrangement: reading `NSScreen` is synchronous in
    /// the callback, so no gap exists and the value rides the event.
    Screens(Vec<Monitor>),
}

/// A placement: the window, where it is, and where to put it.
///
/// `from` orders the writes (grow before move, shrink after); the model owns it, since a
/// placement only fires from a known frame. The payload carries everything performing needs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    pub window: WindowId,
    pub from: Frame,
    pub to: Frame,
}
