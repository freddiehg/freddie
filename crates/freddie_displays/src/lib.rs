//! Display topology for freddie: the set of active displays, and a watcher that
//! reports the full current set on every screen-parameter change and on every wake
//! from sleep.
//!
//! Always the whole set, never a delta, so the consumer's handler is a pure
//! function of topology and a duplicate report is free. macOS posts several
//! screen-parameter notifications for one physical plug; each delivery re-reads
//! the set, and idempotence downstream is what coalesces them.
//!
//! The wake observer exists because a wake can change what is lit without macOS
//! considering the screen parameters changed — and because a consumer that
//! disables a panel wants to re-assert its decision after every wake.
//!
//! # The main thread
//!
//! `NSScreen` and both notification centers belong to the main thread's run loop.
//! [`displays`] must be called on the main thread; [`watch`]'s callback is always
//! delivered there, and `on_change` must hand its work elsewhere and return.
//!
//! macOS only.

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use core_graphics::display::CGDisplay;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSScreen, NSWorkspace,
    NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSNotificationName, NSNumber, NSString,
};

/// A display present according to macOS.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Display {
    /// The `CGDirectDisplayID`, stable for the life of the connection — which is all a consumer
    /// correlates over, since every report carries the full current set.
    pub id: DisplayId,
    /// `CGDisplayIsBuiltin`: whether this is the laptop's own panel.
    pub builtin: bool,
    /// The display's localized name (`NSScreen.localizedName`), which is what `BetterDisplay`'s
    /// `-name=` addresses.
    pub name: String,
}

/// A display's id for correlation within a connection session.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DisplayId(pub u32);

/// The displays currently active. For seeding the initial state; [`watch`] reports changes.
///
/// # Panics
///
/// Panics off the main thread, where `NSScreen` cannot be read.
#[must_use]
pub fn displays() -> Vec<Display> {
    let mtm = MainThreadMarker::new().expect("displays() must run on the main thread");
    read(mtm)
}

/// The current display set, on the main thread.
fn read(mtm: MainThreadMarker) -> Vec<Display> {
    NSScreen::screens(mtm)
        .iter()
        .filter_map(|screen| {
            // `NSScreenNumber` carries the CGDirectDisplayID; a screen without one (which the
            // API contract does not produce, but a callback must be total) is skipped.
            let number = screen
                .deviceDescription()
                .objectForKey(&*NSString::from_str("NSScreenNumber"))?
                .downcast::<NSNumber>()
                .ok()?
                .as_u32();
            Some(Display {
                id: DisplayId(number),
                builtin: CGDisplay::new(number).is_builtin(),
                name: screen.localizedName().to_string(),
            })
        })
        .collect()
}

/// Calls `on_change` with the full current display set on every screen-parameter change and on
/// every wake from sleep.
///
/// Delivery is on the main thread, whichever thread registered, and only while the main thread
/// is inside its run loop. Dropping the returned [`Watcher`] deregisters both observers.
#[must_use = "dropping the watcher deregisters the observers; hold it to keep receiving events"]
pub fn watch<F>(on_change: F) -> Watcher
where
    F: Fn(Vec<Display>) + Send + 'static,
{
    let on_change = Arc::new(on_change);
    // SAFETY: both notification names are immutable extern statics AppKit initializes before
    // any notification can be delivered.
    #[expect(unsafe_code)]
    let (screen_name, wake_name) = unsafe {
        (
            NSApplicationDidChangeScreenParametersNotification,
            NSWorkspaceDidWakeNotification,
        )
    };
    let screens = observe(
        &NSNotificationCenter::defaultCenter(),
        screen_name,
        Arc::clone(&on_change),
    );
    let wake = observe(
        &NSWorkspace::sharedWorkspace().notificationCenter(),
        wake_name,
        on_change,
    );
    Watcher {
        _screens: screens,
        _wake: wake,
    }
}

/// Register one observer on `center` that re-reads the display set and hands it to `on_change`.
fn observe<F>(
    center: &Retained<NSNotificationCenter>,
    name: &'static NSNotificationName,
    on_change: Arc<F>,
) -> Observation
where
    F: Fn(Vec<Display>) + Send + 'static,
{
    let block = RcBlock::new(move |_notif: NonNull<NSNotification>| {
        // Delivery is on the main thread; a delivery anywhere else (which Cocoa does not do)
        // is skipped rather than read from the wrong thread. No panic: this is an FFI frame.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let displays = read(mtm);
        tracing::debug!(?displays, "display topology reported");
        on_change(displays);
    });
    // SAFETY: the block is `Send` because `F` is, which is what makes it sound for Foundation
    // to invoke it on the main thread. `Observation` owns the center, the token, and the block,
    // and removes the observer before either is dropped.
    #[expect(unsafe_code)]
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(name),
            None, // any sender
            None, // no queue: deliver on the posting thread, which is main
            &block,
        )
    };
    Observation {
        center: center.clone(),
        token,
        _block: block,
    }
}

/// A live pair of topology observers (screen parameters, wake). Dropping it deregisters both.
#[must_use = "dropping the watcher deregisters the observers"]
pub struct Watcher {
    _screens: Observation,
    _wake: Observation,
}

/// One registered observer and the center that registered it, held together because
/// deregistering needs that center.
struct Observation {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    /// Held so the callback outlives the observation. The notification center copies the
    /// block, but the closure it wraps is ours to keep alive.
    _block: RcBlock<dyn Fn(NonNull<NSNotification>)>,
}

impl Drop for Observation {
    /// Deregisters the observer; dropping the token alone would leave the center calling a
    /// block whose closure is gone.
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.token).as_ref();
        // SAFETY: `token` is what `addObserverForName…` returned and it is still registered,
        // so this is the documented way to deregister it.
        #[expect(unsafe_code)]
        unsafe {
            self.center.removeObserver(observer);
        }
    }
}

#[cfg(test)]
mod tests {
    // `displays()` needs the main thread and `cargo test` does not provide it; the observers
    // need a run loop it never enters. What can be tested here is nothing; the integration is
    // measured in the consumer.
}
