//! Give the main thread to the platform run loop.
//!
//! `AppKit` delivers its callbacks on the main thread's run loop, and a run loop
//! only delivers while some thread is inside it. So a process that wants to
//! observe `NSWorkspace`, own an `NSStatusItem`, or touch `AppKit` at all must
//! park its main thread in [`MainLoop::run`] and do its real work elsewhere.
//!
//! [`MainLoop::run`] pumps `NSApplication` events (`nextEventMatchingMask` then
//! `sendEvent`), rather than a bare `CFRunLoop`. A bare `CFRunLoop` services
//! run-loop sources, which is enough for `NSWorkspace` notifications, but it never
//! dispatches the window-server `NSEvent`s that a status item's clicks and menu
//! tracking need. Pumping `NSApp` delivers both. Call [`init_menu_bar_app`] once,
//! on the main thread, before creating a status item.
//!
//! ```no_run
//! let (main_loop, stopper) = freddie_main_loop::main_loop();
//!
//! std::thread::spawn(move || {
//!     let _stopper = stopper; // dropping it stops main, however we leave
//!     // ... all of the program's work, on this thread ...
//! });
//!
//! main_loop.run(); // returns once the worker drops the stopper
//! ```
//!
//! This crate is not a source. Registering an `AppKit` observer is somebody else's
//! job (see `freddie_app_nav`), and any thread may do it: registration thread is
//! irrelevant, delivery is always on main. This crate only owns the one thing
//! that cannot be shared, which is the main thread itself.
//!
//! macOS only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use core_foundation::base::TCFType;
use core_foundation::runloop::CFRunLoop;
use objc2::MainThreadMarker;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

/// How long the main thread stays inside the run loop before surfacing to check
/// whether it has been stopped.
///
/// This bounds shutdown latency, not event latency. Sources are serviced the
/// instant they fire, from inside the run loop; the slice only decides how long
/// a stopped loop can take to notice. It exists because [`CFRunLoop::stop`]
/// against a loop that has not started yet is a no-op, so a worker that fails
/// before [`MainLoop::run`] is reached would otherwise leave main asleep forever.
const SLICE: Duration = Duration::from_millis(100);

/// Initializes `NSApplication` as an accessory (menu-bar) app. Call once, on the
/// main thread, before creating any status item.
///
/// Accessory policy keeps the process out of the Dock and the cmd-tab switcher,
/// which is what a menu-bar-only app wants. `finishLaunching` posts the launch
/// notifications `AppKit` expects before it delivers events; `[NSApp run]` would do
/// this, but [`MainLoop::run`] pumps the loop itself, so it is done here.
///
/// Separate from [`main_loop`] on purpose: the tests run off the main thread and
/// call `main_loop` to exercise the stop machinery; they must not touch `NSApp`.
///
/// # Panics
///
/// Panics if called off the main thread.
pub fn init_menu_bar_app() {
    let mtm = MainThreadMarker::new().expect("init_menu_bar_app must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
}

/// Creates the main loop and the handle that stops it.
///
/// Call this on the main thread, before spawning the worker that takes the
/// [`Stopper`].
pub fn main_loop() -> (MainLoop, Stopper) {
    let stop = Arc::new(AtomicBool::new(false));
    let main_loop = MainLoop {
        stop: Arc::clone(&stop),
    };
    let stopper = Stopper {
        stop,
        run_loop: CFRunLoop::get_main(),
    };
    (main_loop, stopper)
}

/// The main thread's run loop, waiting to be run.
#[must_use = "the main loop does nothing until it is run"]
pub struct MainLoop {
    stop: Arc<AtomicBool>,
}

impl MainLoop {
    /// Runs the main thread's run loop until the [`Stopper`] is dropped.
    ///
    /// While this is running, `AppKit` sources registered against the main run loop
    /// deliver their callbacks, on this thread, one at a time. A slow callback
    /// stalls every other source, so callbacks should hand their work to another
    /// thread and return.
    ///
    /// # Panics
    ///
    /// Panics if called off the main thread, where it would block forever without
    /// ever delivering an `AppKit` callback.
    pub fn run(self) {
        assert!(
            is_main_thread(),
            "MainLoop::run must be called on the main thread: AppKit delivers only there"
        );
        let mtm = MainThreadMarker::new().expect("run is on the main thread; asserted above");
        let app = NSApplication::sharedApplication(mtm);
        while !self.stop.load(Ordering::Acquire) {
            autoreleasepool(|_| {
                // One event per slice, dequeued and dispatched. This is the pump a bare
                // CFRunLoop was missing: `nextEventMatchingMask` pulls a window-server
                // event and `sendEvent` delivers it, so a status-item click reaches the
                // menu. The SLICE deadline bounds how long a stopped loop takes to notice;
                // the `Stopper` also breaks the run loop, and the deadline is the floor.
                #[expect(unsafe_code)]
                // SAFETY: on the main thread; dequeuing one event with a 100ms deadline.
                let event = unsafe {
                    let deadline = NSDate::dateWithTimeIntervalSinceNow(SLICE.as_secs_f64());
                    app.nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&deadline),
                        NSDefaultRunLoopMode,
                        true,
                    )
                };
                if let Some(event) = event {
                    app.sendEvent(&event);
                }
            });
        }
    }
}

/// Stops the main loop when dropped, from any thread.
///
/// Hand this to the thread doing the real work. Dropping it, whether by returning
/// normally, returning early on an error, or unwinding from a panic, stops the
/// main loop and lets `main` return. That is what makes the process exit rather
/// than hang with a dead worker, and it beats `process::exit`, which runs no
/// destructors.
pub struct Stopper {
    stop: Arc<AtomicBool>,
    run_loop: CFRunLoop,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        // The flag first: `CFRunLoop::stop` against a loop that has not started is
        // a no-op, so a worker that dies before `MainLoop::run` is reached must
        // leave something behind for it to find.
        self.stop.store(true, Ordering::Release);
        // And the stop, to break it out of the current slice if it has started.
        self.run_loop.stop();
    }
}

/// Whether this is the main thread, asked without `libc` or objc2: only the main
/// thread's run loop is the main run loop.
fn is_main_thread() -> bool {
    CFRunLoop::get_current().as_concrete_TypeRef() == CFRunLoop::get_main().as_concrete_TypeRef()
}

#[cfg(test)]
mod tests {
    use super::{Stopper, is_main_thread, main_loop};

    #[test]
    fn the_test_thread_is_not_main() {
        // libtest runs each test on a spawned thread, which is what lets the
        // off-main assertion below be tested at all.
        assert!(!is_main_thread());
    }

    #[test]
    fn run_off_the_main_thread_panics() {
        let (main_loop, _stopper) = main_loop();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| main_loop.run()));
        assert!(panicked.is_err(), "run() off main must panic, not hang");
    }

    // Dropping the stopper before the loop runs must still stop it. The flag is
    // what carries that, since CFRunLoopStop against a loop that has not started
    // is a no-op. Asserted on the flag rather than by running a loop, because
    // `cargo test` has no main thread to give.
    #[test]
    fn dropping_the_stopper_sets_the_flag() {
        let (main_loop, stopper) = main_loop();
        assert!(!main_loop.stop.load(std::sync::atomic::Ordering::Acquire));
        drop(stopper);
        assert!(main_loop.stop.load(std::sync::atomic::Ordering::Acquire));
    }

    // The stopper is the thing that crosses a thread boundary; the main loop is
    // the thing that must not.
    #[test]
    fn the_stopper_is_send() {
        const fn assert_send<T: Send>() {}
        assert_send::<Stopper>();
    }
}
