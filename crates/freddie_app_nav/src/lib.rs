//! App navigation for freddie: bring an app to the front, and watch which app is
//! frontmost.
//!
//! Two directions, both over plain app names (whatever the OS calls the app, e.g.
//! `"Google Chrome"`):
//!
//! - [`foreground`] performs the sink half: it asks the OS to make a named app
//!   frontmost, launching it if needed. Fire-and-forget; it does not report back.
//! - [`watch`] is the source half: it runs a background watcher that calls a
//!   callback with the frontmost app's name, once at startup and again on every
//!   change. That is the event the model dispatches.
//!
//! The two are decoupled on purpose (see `refactors/pending/event-loop.md`):
//! [`foreground`] triggers a change, [`watch`] reports the change that actually
//! happens, and nothing ties one call to the other. The name-to-app mapping is the
//! consumer's (mercury owns its `App` enum), so this crate stays app-agnostic and
//! only ever hands up a string.
//!
//! macOS only. Foregrounding shells out to `open -a`, and the watcher polls the
//! frontmost process name via `osascript`. Both are the cheap userland path; the
//! `NSWorkspace`/`active-win-pos-rs` upgrades noted in
//! `refactors/pending/foreground-events.md` and `app-foregrounding.md` slot in
//! behind the same API.

use std::fmt;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A sensible poll cadence for the polling backend, for callers that do not have
/// a preference: fast enough to feel immediate, slow enough to stay idle.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// Brings the named app to the front, launching it if it is not running.
///
/// This is fire-and-forget: it asks the OS and returns. It does not confirm the
/// app actually came up; the [`watch`] source reports the real frontmost app, so
/// the consumer never has to trust that this succeeded.
///
/// # Errors
///
/// Returns [`NavError::Spawn`] if `open` cannot be spawned, or [`NavError::Failed`]
/// if `open` runs but exits non-zero (unknown app, activation refused).
pub fn foreground(app_name: &str) -> Result<(), NavError> {
    let status = Command::new("open")
        .args(open_args(app_name))
        .status()
        .map_err(NavError::Spawn)?;
    if status.success() {
        Ok(())
    } else {
        Err(NavError::Failed)
    }
}

/// The `open` arguments that foreground `app_name`: `open -a <app_name>`, which
/// launches the app if needed and brings it to the front.
const fn open_args(app_name: &str) -> [&str; 2] {
    ["-a", app_name]
}

/// Starts watching the frontmost app, calling `on_change` with its name: once
/// immediately with whatever is frontmost now, then again on every change.
///
/// The callback runs on the watcher's own thread. Returns a [`Watcher`] that stops
/// the thread when dropped, so hold onto it for as long as you want the events.
#[must_use = "the watcher stops as soon as it is dropped; hold it to keep receiving events"]
pub fn watch<F>(poll_interval: Duration, on_change: F) -> Watcher
where
    F: FnMut(&str) + Send + 'static,
{
    Watcher::spawn(poll_interval, frontmost, on_change)
}

/// A running frontmost-app watcher. Dropping it stops the background thread.
#[must_use = "the watcher stops as soon as it is dropped"]
pub struct Watcher {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Spawns the watcher thread over a `query` source (the OS in production, a
    /// fake in tests) and a change callback.
    fn spawn<Q, F>(poll_interval: Duration, mut query: Q, mut on_change: F) -> Self
    where
        Q: FnMut() -> Option<String> + Send + 'static,
        F: FnMut(&str) + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = Arc::clone(&running);
        let handle = thread::spawn(move || {
            let mut poller = Poller::new();
            while running_thread.load(Ordering::Relaxed) {
                if let Some(app) = poller.observe(query()) {
                    // The raw name, before the consumer maps it onto its own apps.
                    tracing::debug!(app = %app, "frontmost app changed");
                    on_change(&app);
                }
                responsive_sleep(poll_interval, &running_thread);
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    /// Stops the watcher and waits for its thread to finish.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Sleeps up to `total`, but wakes every `STEP` to check `running`, so a stopped
/// watcher does not linger for the whole poll interval before exiting.
fn responsive_sleep(total: Duration, running: &AtomicBool) {
    const STEP: Duration = Duration::from_millis(50);
    let mut slept = Duration::ZERO;
    while slept < total && running.load(Ordering::Relaxed) {
        let chunk = STEP.min(total.saturating_sub(slept));
        thread::sleep(chunk);
        slept += chunk;
    }
}

/// Tracks the last frontmost app so the watcher reports only changes.
struct Poller {
    last: Option<String>,
}

impl Poller {
    const fn new() -> Self {
        Self { last: None }
    }

    /// Records `current` and returns it when it differs from the last reported
    /// app, so the caller fires a change event only on a real change. A `None`
    /// read (the query failed) is ignored and leaves the last app unchanged.
    fn observe(&mut self, current: Option<String>) -> Option<String> {
        match current {
            Some(app) if self.last.as_deref() != Some(app.as_str()) => {
                self.last = Some(app.clone());
                Some(app)
            }
            _ => None,
        }
    }
}

/// The name of the frontmost application, or `None` when it cannot be read.
///
/// Uses `osascript` to ask System Events for the frontmost process name. This is
/// the polling backend; an `NSWorkspace` observer is the event-based upgrade.
fn frontmost() -> Option<String> {
    const SCRIPT: &str = "tell application \"System Events\" to name of first application process whose frontmost is true";
    let output = Command::new("osascript")
        .arg("-e")
        .arg(SCRIPT)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;

    #[test]
    fn open_args_are_dash_a_name() {
        assert_eq!(open_args("Google Chrome"), ["-a", "Google Chrome"]);
        assert_eq!(open_args("Zed"), ["-a", "Zed"]);
    }

    #[test]
    fn poller_reports_the_first_app() {
        let mut p = Poller::new();
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
    }

    #[test]
    fn poller_suppresses_a_repeat() {
        let mut p = Poller::new();
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
        assert_eq!(p.observe(Some("Chrome".to_owned())), None);
    }

    #[test]
    fn poller_reports_each_change() {
        let mut p = Poller::new();
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
        assert_eq!(p.observe(Some("Zed".to_owned())), Some("Zed".to_owned()));
        assert_eq!(p.observe(Some("Zed".to_owned())), None);
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
    }

    #[test]
    fn poller_ignores_failed_reads() {
        let mut p = Poller::new();
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
        // A failed query does not clear the last app, so the next successful read
        // of the same app is still a no-op.
        assert_eq!(p.observe(None), None);
        assert_eq!(p.observe(Some("Chrome".to_owned())), None);
    }

    #[test]
    fn poller_reports_a_change_after_a_failed_read() {
        let mut p = Poller::new();
        assert_eq!(
            p.observe(Some("Chrome".to_owned())),
            Some("Chrome".to_owned())
        );
        assert_eq!(p.observe(None), None);
        assert_eq!(p.observe(Some("Zed".to_owned())), Some("Zed".to_owned()));
    }

    // The watcher, driven off a scripted source instead of the OS: it delivers the
    // current app then each change, and stops cleanly.
    #[test]
    fn watcher_delivers_current_then_changes() {
        // The scripted frontmost app: Chrome, Chrome, Zed, then Zed forever.
        let script = Arc::new(Mutex::new(
            vec!["Chrome", "Chrome", "Zed"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ));
        let idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let query_idx = Arc::clone(&idx);
        let query_script = Arc::clone(&script);
        let query = move || {
            let script = query_script.lock().unwrap();
            let i = query_idx.fetch_add(1, Ordering::Relaxed);
            script.get(i).or_else(|| script.last()).cloned()
        };

        let (tx, rx) = mpsc::channel();
        let watcher = Watcher::spawn(Duration::from_millis(5), query, move |app| {
            let _ = tx.send(app.to_owned());
        });

        // Two changes: Chrome (initial), then Zed. The repeated Chrome and the
        // trailing Zeds are suppressed.
        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        watcher.stop();
        assert_eq!(first, "Chrome");
        assert_eq!(second, "Zed");
        // No third change ever fires (everything after is Zed).
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }
}
