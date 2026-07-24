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
//! The loop is event-driven: it sleeps in `nextEventMatchingMask` with no deadline until a real
//! event or a posted wake, so an idle process does nothing. Work handed to `on_wake` over a
//! [`MainWaker::channel`] wakes the loop on send, and dropping the [`Stopper`] wakes it to exit.
//!
//! ```no_run
//! let (main_loop, stopper, _waker) = freddie_main_loop::main_loop();
//!
//! std::thread::spawn(move || {
//!     let _stopper = stopper; // dropping it stops main, however we leave
//!     // ... all of the program's work, on this thread ...
//! });
//!
//! main_loop.run(|| {}); // returns once the worker drops the stopper
//! ```
//!
//! This crate is not a source. Registering an `AppKit` observer is somebody else's
//! job (see `freddie_app_nav`), and any thread may do it: registration thread is
//! irrelevant, delivery is always on main. This crate only owns the one thing
//! that cannot be shared, which is the main thread itself.
//!
//! macOS only.

use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use core_foundation::base::TCFType;
use core_foundation::runloop::CFRunLoop;
use objc2::MainThreadMarker;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventSubtype, NSEventType,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint};

/// The subtype stamped on the events posted only to wake the loop, so [`is_wake_event`] can tell
/// them from real window-server events.
const WAKE_SUBTYPE: i16 = 1;

/// Initializes `NSApplication` as an accessory (menu-bar) app. Call once, on the
/// main thread, before creating any status item.
///
/// Accessory policy keeps the process out of the Dock and the cmd-tab switcher,
/// which is what a menu-bar-only app wants. `finishLaunching` posts the launch
/// notifications `AppKit` expects before it delivers events; `[NSApp run]` would do
/// this, but [`MainLoop::run`] pumps the loop itself, so it is done here.
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

/// Creates the main loop, the handle that stops it, and the waker that wakes it.
///
/// Call this on the main thread, before spawning the worker that takes the [`Stopper`]. Hand
/// [`MainWaker`] clones to the threads that send on channels [`MainLoop::run`]'s `on_wake` drains.
///
/// # Panics
///
/// Panics if called off the main thread, where `NSApp` is not reachable.
pub fn main_loop() -> (MainLoop, Stopper, MainWaker) {
    let mtm = MainThreadMarker::new().expect("main_loop must be called on the main thread");
    let waker = MainWaker {
        app_handle: AppHandle::new(&NSApplication::sharedApplication(mtm)),
    };
    let (stop_signal, stop_receiver) = std::sync::mpsc::channel();
    (
        MainLoop { stop_receiver },
        Stopper {
            stop_signal,
            waker: waker.clone(),
        },
        waker,
    )
}

/// A handle to the process `NSApplication`, for posting a wake event from any thread.
///
/// `NSApp` outlives the process, so the pointer is stable, and `postEvent:atStart:` is thread-safe,
/// so a `&` call off the main thread is sound. Built on the main thread, where `NSApp` is reachable.
#[derive(Clone)]
struct AppHandle(NonNull<NSApplication>);

// SAFETY: `NSApp` lives for the whole process and `postEvent:atStart:` is thread-safe, so the
// pointer stays valid and the call is sound from any thread.
#[expect(unsafe_code)]
unsafe impl Send for AppHandle {}
#[expect(unsafe_code)]
unsafe impl Sync for AppHandle {}

impl AppHandle {
    fn new(app: &NSApplication) -> Self {
        Self(NonNull::from(app))
    }

    /// Post an application-defined event, which `nextEventMatchingMask` dequeues and returns.
    fn post_wake_event(&self) {
        #[expect(unsafe_code)]
        // SAFETY: `self.0` is the process `NSApp`, valid for the process; `postEvent:atStart:` is
        // thread-safe; the event is a fresh application-defined event carrying `WAKE_SUBTYPE`.
        unsafe {
            let app = self.0.as_ref();
            let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::ApplicationDefined,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                0.0,
                0,
                None,
                WAKE_SUBTYPE,
                0,
                0,
            );
            if let Some(event) = event {
                app.postEvent_atStart(&event, true);
            }
        }
    }
}

/// Wakes the main loop from any thread, so work queued for `on_wake` runs now. Cheap to clone; give
/// one to each thread that sends on a channel `on_wake` drains.
#[derive(Clone)]
pub struct MainWaker {
    app_handle: AppHandle,
}

impl MainWaker {
    /// Wake the main loop: `nextEventMatchingMask` returns and `on_wake` runs.
    ///
    /// Send on the channel first, then call this: the send has to be visible when `on_wake` drains.
    pub fn wake(&self) {
        self.app_handle.post_wake_event();
    }

    /// A channel whose sender wakes the loop on every send. The receiver is a plain
    /// `std::sync::mpsc::Receiver`, drained in `on_wake` on the main thread.
    #[must_use]
    pub fn channel<T>(&self) -> (WakingSender<T>, Receiver<T>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            WakingSender {
                sender,
                waker: self.clone(),
            },
            receiver,
        )
    }
}

/// A channel sender that wakes the main loop after each send, so the value reaches `on_wake` at once.
///
/// `Send` and `Clone`. A consumer holds one of these instead of a bare `Sender`, so it cannot
/// send without waking; that is enforced here rather than left to each send site to remember.
pub struct WakingSender<T> {
    sender: Sender<T>,
    waker: MainWaker,
}

// Hand-written, not derived: `Sender<T>` is `Clone` for every `T` (a clone duplicates the handle,
// not a message), so this must be too. `#[derive(Clone)]` would add a spurious `T: Clone` bound,
// which `std`'s own `impl<T> Clone for Sender<T>` also avoids.
impl<T> Clone for WakingSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<T> WakingSender<T> {
    /// Send, then wake the main loop.
    ///
    /// # Errors
    ///
    /// If the receiver has been dropped, the same as [`std::sync::mpsc::Sender::send`].
    pub fn send(&self, value: T) -> Result<(), std::sync::mpsc::SendError<T>> {
        self.sender.send(value)?;
        self.waker.wake();
        Ok(())
    }
}

/// The main thread's run loop, waiting to be run.
#[must_use = "the main loop does nothing until it is run"]
pub struct MainLoop {
    stop_receiver: Receiver<()>,
}

impl MainLoop {
    /// Runs the main thread's run loop until the [`Stopper`] is dropped.
    ///
    /// While this is running, `AppKit` sources registered against the main run loop
    /// deliver their callbacks, on this thread, one at a time. A slow callback
    /// stalls every other source, so callbacks should hand their work to another
    /// thread and return.
    ///
    /// `on_wake` runs on each pass: after a real event, and after a posted wake from a
    /// [`WakingSender`] or the [`Stopper`]. It runs ON the main thread, so it is where a caller does
    /// main-thread-only work that came from elsewhere. It must return promptly, for the same reason
    /// a source callback must.
    ///
    /// # Panics
    ///
    /// Panics if called off the main thread, where it would block forever without
    /// ever delivering an `AppKit` callback.
    pub fn run(self, mut on_wake: impl FnMut()) {
        assert!(
            is_main_thread(),
            "MainLoop::run must be called on the main thread: AppKit delivers only there"
        );
        let mtm = MainThreadMarker::new().expect("run is on the main thread; asserted above");
        let app = NSApplication::sharedApplication(mtm);
        // `Empty` is the only reason to keep turning. A buffered `()` means the stopper sent
        // before dropping; `Disconnected` means it dropped without sending, which cannot happen
        // while its `Drop` sends, but is still a stop.
        while matches!(self.stop_receiver.try_recv(), Err(TryRecvError::Empty)) {
            autoreleasepool(|_| {
                // Block until something happens: a real window-server event, or a posted wake. No
                // deadline, so an idle loop sleeps at no cost.
                #[expect(unsafe_code)]
                // SAFETY: on the main thread; dequeuing one event, waiting indefinitely.
                let event = unsafe {
                    app.nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&NSDate::distantFuture()),
                        NSDefaultRunLoopMode,
                        true,
                    )
                };
                if let Some(event) = event {
                    // A wake event exists only to break the wait; do not dispatch it. Everything
                    // else is a real event the status item and menu tracking need.
                    if !is_wake_event(&event) {
                        app.sendEvent(&event);
                    }
                }
                on_wake();
            });
        }
    }
}

/// Stops the main loop when dropped, from any thread.
///
/// Hand this to the thread doing the real work. Dropping it, whether by returning
/// normally or returning early on an error, stops the main loop and lets `main` return. That is
/// what makes the process exit rather than hang with a dead worker. A panic does not reach here;
/// it aborts the process from the panic hook (see `freddie_cli`'s `log_panics`).
///
/// A `Stopper` is a [`MainWaker`] plus the exit signal: dropping it wakes the loop and tells it to
/// stop, where a bare wake tells it to keep going.
pub struct Stopper {
    stop_signal: Sender<()>,
    waker: MainWaker,
}

impl Drop for Stopper {
    fn drop(&mut self) {
        // Signal first, then wake, the same ordering as any waking send: the loop must see the stop
        // when the posted event breaks its wait. A post that lands before the loop blocks is not
        // lost (it sits in the event queue), so there is no deadline backstop and none is needed.
        let _ = self.stop_signal.send(());
        self.waker.wake();
    }
}

/// Whether this is the main thread, asked without `libc` or objc2: only the main
/// thread's run loop is the main run loop.
fn is_main_thread() -> bool {
    CFRunLoop::get_current().as_concrete_TypeRef() == CFRunLoop::get_main().as_concrete_TypeRef()
}

/// Whether this is one of the events posted only to wake the loop.
fn is_wake_event(event: &NSEvent) -> bool {
    event.r#type() == NSEventType::ApplicationDefined
        && event.subtype() == NSEventSubtype(WAKE_SUBTYPE)
}

#[cfg(test)]
mod tests {
    use super::{MainLoop, Stopper, is_main_thread};
    use std::sync::mpsc::{TryRecvError, channel};

    #[test]
    fn the_test_thread_is_not_main() {
        // libtest runs each test on a spawned thread, which is what lets the
        // off-main assertion below be tested at all.
        assert!(!is_main_thread());
    }

    #[test]
    fn run_off_the_main_thread_panics() {
        // A `MainLoop` built directly, since `main_loop` needs the main thread for `NSApp` and the
        // point is to reach `run` off main.
        let (_stop_signal, stop_receiver) = channel::<()>();
        let main_loop = MainLoop { stop_receiver };
        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| main_loop.run(|| {})));
        assert!(panicked.is_err(), "run() off main must panic, not hang");
    }

    // A stop reaches the loop as a channel send that `run`'s condition reads. The wake that follows
    // it is an `NSApp` post, which needs the main thread and is covered by the spike, so this
    // asserts the signal half by driving the channel a `Stopper` would.
    #[test]
    fn a_stop_signal_reaches_the_loop() {
        let (stop_signal, stop_receiver) = channel::<()>();
        let main_loop = MainLoop { stop_receiver };
        assert!(matches!(
            main_loop.stop_receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));
        let _ = stop_signal.send(());
        assert!(main_loop.stop_receiver.try_recv().is_ok());
    }

    // The stopper is the thing that crosses a thread boundary; the main loop is
    // the thing that must not.
    #[test]
    fn the_stopper_is_send() {
        const fn assert_send<T: Send>() {}
        assert_send::<Stopper>();
    }
}
