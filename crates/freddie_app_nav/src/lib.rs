//! App navigation for freddie: bring an app to the front, and watch which app is
//! frontmost.
//!
//! Both directions speak bundle identifiers (`com.google.Chrome`), which are the
//! stable name for an app. Display names are not: System Events calls Ghostty
//! `ghostty` while the app calls itself `Ghostty`.
//!
//! - [`foreground`] is the sink: it asks the OS to bring an app to the front,
//!   launching it if needed. Fire-and-forget; it does not report back.
//! - [`watch`] is the source. It observes `NSWorkspace`'s
//!   `didActivateApplication` notification, so the callback runs once per real
//!   activation. No polling and no interval.
//!
//! The two are decoupled on purpose (see `refactors/past/event-loop.md`):
//! [`foreground`] asks for a change, [`watch`] reports the change that actually
//! happened, and nothing ties one call to the other. The bundle-id-to-app mapping
//! belongs to the consumer (which owns its `App` enum), so this crate only ever
//! hands up a string.
//!
//! # The main thread
//!
//! `NSWorkspace` registers its notification port with the main thread's run loop
//! and gives no handle to redirect it, so a callback only ever runs while the main
//! thread is inside that loop. `freddie_main_loop` is how you get there, and the
//! binary is what calls it. This crate registers a source and nothing else.
//!
//! Registering from any thread is fine. Delivery is always on main, and
//! main-thread callbacks are serialized, so `on_change` must do its work
//! elsewhere and return.
//!
//! macOS only.

use std::fmt;
use std::process::Command;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::NSNotification;

/// Foregrounding an app failed.
#[derive(Debug)]
pub enum NavError {
    /// `open` could not be spawned at all.
    Spawn(std::io::Error),
    /// `open` ran but reported failure (the app is missing, or activation was
    /// refused).
    Failed,
}

impl fmt::Display for NavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not run `open`: {e}"),
            Self::Failed => {
                f.write_str("`open` reported failure (app missing or activation refused)")
            }
        }
    }
}

impl std::error::Error for NavError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(e) => Some(e),
            Self::Failed => None,
        }
    }
}

/// Brings the app with this bundle identifier to the front, launching it if it is
/// not running.
///
/// Fire-and-forget: it asks the OS and returns. It does not confirm the app came
/// up; [`watch`] reports the real frontmost app, so the consumer never has to
/// trust that this succeeded.
///
/// # Errors
///
/// Returns [`NavError::Spawn`] if `open` cannot be spawned, or [`NavError::Failed`]
/// if `open` exits non-zero (unknown bundle id, activation refused).
pub fn foreground(bundle_id: &str) -> Result<(), NavError> {
    let status = Command::new("open")
        .args(open_args(bundle_id))
        .status()
        .map_err(NavError::Spawn)?;
    if status.success() {
        Ok(())
    } else {
        Err(NavError::Failed)
    }
}

/// The `open` arguments that foreground `bundle_id`: `open -b <bundle_id>`, which
/// launches the app if needed and brings it to the front.
const fn open_args(bundle_id: &str) -> [&str; 2] {
    ["-b", bundle_id]
}

/// The bundle identifier of the frontmost app, or `None` if there is none.
///
/// Good for seeding the initial state, and for nothing else. It reads a cache that
/// the workspace notification machinery refreshes, so polling it in a loop returns
/// the app that was frontmost at process start, forever. Use [`watch`] for changes.
#[must_use]
pub fn frontmost() -> Option<String> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .bundleIdentifier()
        .map(|id| id.to_string())
}

/// The bundle identifier of the app a `didActivateApplication` notification is
/// about.
fn activated_bundle_id(notif: &NSNotification) -> Option<String> {
    let info = notif.userInfo()?;
    // SAFETY: `NSWorkspaceApplicationKey` is an immutable extern static `NSString`
    // that AppKit initializes before any notification can be delivered.
    #[expect(unsafe_code)]
    let key = unsafe { NSWorkspaceApplicationKey };
    let app = info
        .objectForKey(key)?
        .downcast::<NSRunningApplication>()
        .ok()?;
    app.bundleIdentifier().map(|id| id.to_string())
}

/// Calls `on_change` with the bundle identifier of each app as it becomes
/// frontmost.
///
/// One call per real activation: `NSWorkspace` posts the notification only when the
/// front app actually changes, so there is nothing to diff and no interval to tune.
///
/// This reports changes, not the state at registration. Seed the current app with
/// [`frontmost`].
///
/// The callback runs on the main thread, whichever thread registered it, and only
/// while the main thread is inside its run loop.
///
/// Dropping the returned [`Watcher`] deregisters the observer. Leaking it instead
/// leaves the callback live and callable after whatever it captured is gone.
#[must_use = "dropping the watcher deregisters the observer; hold it to keep receiving events"]
pub fn watch<F>(on_change: F) -> Watcher
where
    F: Fn(&str) + Send + 'static,
{
    let block = RcBlock::new(move |notif: NonNull<NSNotification>| {
        // SAFETY: Foundation hands the block a valid, retained notification, live
        // for the duration of this call.
        #[expect(unsafe_code)]
        let notif = unsafe { notif.as_ref() };
        if let Some(bundle_id) = activated_bundle_id(notif) {
            tracing::debug!(app = %bundle_id, "frontmost app changed");
            on_change(&bundle_id);
        }
    });

    // SAFETY: `NSWorkspaceDidActivateApplicationNotification` is an immutable
    // extern static. The block is `Send` because `F` is, which is what makes it
    // sound for Foundation to invoke it on the main thread. `Watcher` owns both the
    // token and the block, and removes the observer before either is dropped.
    #[expect(unsafe_code)]
    let token = unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None, // any sender
                None, // no queue: deliver on the posting thread, which is main
                &block,
            )
    };

    Watcher {
        token,
        _block: block,
    }
}

/// A live `didActivateApplication` observer. Dropping it deregisters.
#[must_use = "dropping the watcher deregisters the observer"]
pub struct Watcher {
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    /// Held so the callback outlives the observation. The notification center
    /// copies the block, but the closure it wraps is ours to keep alive.
    _block: RcBlock<dyn Fn(NonNull<NSNotification>)>,
}

impl Drop for Watcher {
    /// Deregisters the observer. This is the only way to stop one.
    ///
    /// Dropping the token alone does not stop the observation: the notification
    /// center goes on calling the block, which is a use-after-free once the closure
    /// is gone. `removeObserver` is what stops it, and Cocoa requires it before the
    /// observer is deallocated.
    ///
    /// Measured to work off the main thread, which is where this runs: the
    /// `Watcher` lives on the thread that registered it.
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.token).as_ref();
        // SAFETY: `token` is what `addObserverForName...` returned and it is still
        // registered, so this is the documented way to deregister it.
        #[expect(unsafe_code)]
        unsafe {
            NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .removeObserver(observer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{frontmost, open_args};

    #[test]
    fn open_args_are_dash_b_bundle_id() {
        assert_eq!(open_args("com.google.Chrome"), ["-b", "com.google.Chrome"]);
        assert_eq!(open_args("dev.zed.Zed"), ["-b", "dev.zed.Zed"]);
    }

    /// `frontmost` reads a real `NSWorkspace`, so it runs under `cargo test` and
    /// reports whatever is frontmost. It must not panic, and what it returns must
    /// look like a bundle id.
    #[test]
    fn frontmost_is_a_bundle_id_or_nothing() {
        if let Some(id) = frontmost() {
            assert!(!id.is_empty());
            assert!(id.contains('.'), "not a bundle id: {id}");
        }
    }

    // The observer cannot be tested here: `cargo test` never puts the main thread
    // in a run loop, so nothing is ever delivered. What was measured out of process
    // is recorded in `refactors/past/foreground-events.md`.
}
