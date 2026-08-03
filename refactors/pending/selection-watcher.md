# the selection watcher

Two freddie changes: the per-app observer scaffolding extracted from `freddie_windows`, and `freddie_selection` built on it. The figaro consumer half — the map on the model, the event, the projection — is `figaro/refactors/pending/read-selection.md`, which lands after this doc and after `figaro/refactors/pending/sync-fixes.md`'s pid mirror.

## Change 1 (prefactor): `freddie_ax_observer`

The per-app observer scaffolding moves out of `freddie_windows` into a shared crate, because `freddie_selection` needs machinery that is line-for-line the same: one `AXObserver` per observable app, created at install and at every launch, torn down at termination, its run loop source on the main thread, its `refcon` a boxed registration freed exactly when the observer is released.

`crates/freddie_ax_observer`, carrying the `unsafe_code = "deny"`-with-`#[expect]` lint table (it is an AX boundary crate). What moves in, verbatim except for the genericized seams:

- `Pid` and `ObservableApp` (with `ObservableApp::of`, the UI-service filter).
- `Observation`, `observe_notification`, `notified_app`: the `NSNotificationCenter` registration plumbing.
- `add_notification`: the logged-and-skipped single registration.
- The launch/terminate halves of `watch_notifications`, as the crate's own wiring.
- The lifecycle skeleton of `observe_app` (create the app element, `AXObserverCreate`, add the run loop source, insert into the per-app map) and the teardown ordering of `AppObserver::drop` and `forget_app` (run loop source removed before the registration box drops; observer dropped before the consumer hears the app is gone).

The public surface:

```rust
/// One app the watcher can see: the observer to register notifications on, and the app
/// element they are registered against. Borrowed for the duration of one callback.
pub struct AppSeen {
    pub pid: Pid,
    pub observer: AXObserverRef,
    pub app_element: AXUIElementRef,
}

/// One `AXObserver` per observable app, kept across launches and terminations.
///
/// `R` is the consumer's per-app registration: built once per app, boxed here so its address
/// is stable for the life of that app's observer, handed to the consumer's registrations as
/// the `refcon`, and freed when the observer is released. `!Send`: main thread only, like the
/// `Watcher` this was extracted from.
pub struct AppWatch<R> { /* apps: HashMap<Pid, (observer, Box<R>)>, _notifications: Vec<Observation> */ }

/// Observe every running observable app now and every one that launches later.
///
/// `callback` is the consumer's C notification callback (its `refcon` is the `&R` for that
/// app). `on_app` runs once per observed app — at install for the running set, at launch for
/// the rest — and is where the consumer registers its notifications and seeds; it receives
/// the stable `refcon` pointer for those registrations. `on_app_gone` runs after the app's
/// observer and registration are torn down.
pub fn watch_apps<R: 'static>(
    callback: unsafe extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void),
    make_registration: impl Fn(&AppSeen) -> R + 'static,
    on_app: impl Fn(&AppSeen, *mut c_void) + 'static,
    on_app_gone: impl Fn(Pid) + 'static,
) -> AppWatch<R>;
```

`freddie_windows` after: its `Registration` is the `R`; `observe_app` shrinks to its window half (register the three window notifications, walk `app_windows`), living in `on_app`; `forget_app` keeps only the `Closed` reporting, in `on_app_gone`; the activation and screens observations stay in `freddie_windows`, built on the crate's `observe_notification`. `WindowChange`, `Elements`, `Snapshot`, the placement path, and `on_notification`'s window logic do not move. Behavior-preserving: the same observers fire the same callbacks in the same order.

## Change 2: `freddie_selection`

`crates/freddie_selection`, beside `freddie_windows`: the Accessibility calls are raw C, so they live in a freddie platform crate behind safe functions, per `docs/platform-apis.md`. The workspace `members` list gains the crate.

```toml
# crates/freddie_selection/Cargo.toml
[package]
name = "freddie_selection"
description = "Every app's selected text, for freddie (the macOS Accessibility watcher)."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
freddie_ax_observer = { path = "../freddie_ax_observer" }
accessibility-sys = "0.1"
core-foundation = { version = "0.10", features = ["link"] }
tracing = "0.1"

# Not `workspace = true`: the workspace forbids `unsafe_code`, and `forbid` cannot
# be relaxed from inside the crate. The Accessibility API is raw C, so every call
# is unsafe and allowed at its site with a SAFETY comment. Every other lint
# matches the workspace, and every other crate keeps the `forbid`.
[lints.rust]
unsafe_code = "deny"

[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery = { level = "deny", priority = -1 }
cargo = { level = "deny", priority = -1 }
multiple_crate_versions = "allow"
cargo_common_metadata = "allow"
missing_const_for_fn = "allow"
# Matches the workspace table, which this crate does not inherit.
empty_structs_with_brackets = "deny"
```

The answer type, and the re-export figaro keys its map by:

```rust
pub use freddie_ax_observer::Pid;

/// What an app's focused element said when asked for its selected text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Selection {
    /// The selected text as the app reports it. Never empty: an empty answer is [`Empty`](Self::Empty).
    Text(String),
    /// The element answers the question, and nothing is selected.
    Empty,
    /// There is no focused element, or the focused element does not expose its selection
    /// through Accessibility.
    Unsupported,
}

/// How long the watcher's re-read of an app may take before it is abandoned. A selection
/// notification carries no text, so learning the new value is one `kAXSelectedText` read into
/// the app that changed — the sync's transport, the way the window watcher re-reads a frame on
/// every move notification. The read runs on the watcher's worker thread, so a hung app costs
/// this bound there and nothing on the main loop. Set on the focused element only, so the
/// process-global timeout every other AX caller in the process runs under stays untouched.
const AX_TIMEOUT_SECONDS: f32 = 1.0;
```

The watcher, on the scaffolding:

```rust
/// The generation of one app's sync: minted by the watcher, incremented on every change
/// notification for that app. A re-read carries the generation of the change that queued it,
/// which is what makes a stale read harmless: it names a generation the entry has moved past.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelectionGen(u64);

/// One app's place in the sync: either the last read answer, or the gap between a change
/// notification and its re-read landing. The gap is a state, not a stale value: the moment the
/// notification fires, the old answer is known to be wrong, and showing it would be lying — a
/// consumer treats `Pending` as "no selection right now".
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SelectionEntry {
    /// The re-read for this generation is in flight: the selection changed and there is no
    /// current answer yet.
    Pending(SelectionGen),
    /// What the last re-read answered.
    Known(Selection),
}

/// What the selection map should learn. One variant per thing the watcher can tell you.
#[derive(Clone, PartialEq, Eq)]
pub enum SelectionChange {
    /// A change notification fired: the entry becomes [`SelectionEntry::Pending`] at this
    /// generation, and a re-read is queued carrying it.
    Changed(Pid, SelectionGen),
    /// The re-read queued at this generation landed. Applied only if the entry still holds
    /// `Pending` at the same generation; anything else means a newer change superseded the
    /// read, and the answer describes a selection that no longer exists.
    Reported(Pid, SelectionGen, Selection),
    /// The app is gone: remove the entry.
    AppGone(Pid),
}

impl fmt::Debug for SelectionChange {
    /// `Reported` without its payload: a change dispatches into the model, so its `Debug` is
    /// what the dispatch record prints, and the record should say a read landed, not carry the
    /// text it landed with.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed(pid, gen) => write!(f, "Changed({pid:?}, {gen:?})"),
            Self::Reported(pid, gen, _) => write!(f, "Reported({pid:?}, {gen:?})"),
            Self::AppGone(pid) => write!(f, "AppGone({pid:?})"),
        }
    }
}

/// Watch every app's selection. Returns the seed — every observed app as
/// [`SelectionEntry::Pending`], its first read already queued — and the watch that keeps
/// `on_change` fed. Installed on the main thread, like `freddie_windows::watch`; `on_change`
/// is called from the watcher's threads.
pub fn watch(
    on_change: impl Fn(SelectionChange) + Send + 'static,
) -> (HashMap<Pid, SelectionEntry>, SelectionWatch);
```

Internally, the sync is two-phase so the notification callback never waits on the app:

- The main-thread callback, for either notification, mints the pid's next generation, reports `Changed(pid, gen)`, and sends `(pid, gen)` down a channel to the watcher's worker thread. `on_app` does the same for its registration pass, which is also the seed path: an app enters the map as `Pending` at its first generation and its first `Reported` follows. `on_app_gone` reports `AppGone`. The per-pid counters are the callback's own table, main-thread state like the rest of the watcher.
- The worker owns the receiving end. For each `(pid, gen)` it drains — dropping queued pairs a later generation for the same pid has superseded, so a drag's burst of notifications costs one read — it runs `current_selection(pid)` (below) and reports `Reported(pid, gen, selection)`.

The generation is what makes the interleavings safe without any claim about scheduling: a notification firing mid-read produces `Changed(pid, n+1)` before `Reported(pid, n, …)` can land, the model sees the generations disagree and drops the stale answer, and the read queued at `n+1` delivers the real one.

`SelectionWatch` holds the `AppWatch` and the worker's sender; dropping it closes the channel, which ends the worker.

The sync's transport, private to the worker and exported to nobody: a notification carries no text, so this one bounded read is how a change becomes a value. Nothing outside the watcher can call it; the only read surface any consumer has is the model's state.

```rust
/// What `pid`'s focused element answers right now. The worker's half of one sync step: the
/// notification said the selection changed, this learns what it changed to. One synchronous
/// round-trip into the app, on the worker thread; the app element is created here from the
/// pid, so nothing AX crosses a thread.
#[must_use]
fn current_selection(pid: Pid) -> Selection {
    // SAFETY: `pid` names a process; the element is +1, released with the `Owned`.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid.0) };
    let Some(app) = Owned::new(app.cast()) else {
        return Selection::Unsupported;
    };
    let Some(focused) = copy_attribute(element(&app), kAXFocusedUIElementAttribute) else {
        return Selection::Unsupported;
    };
    // Best effort: an app that never answers should cost one second, not the default six. A
    // refusal to set the timeout changes the bound, not the answer, so the status is dropped.
    // SAFETY: the element is live for the duration of the call.
    #[expect(unsafe_code)]
    unsafe {
        AXUIElementSetMessagingTimeout(element(&focused), AX_TIMEOUT_SECONDS);
    }
    let Some(value) = copy_attribute(element(&focused), kAXSelectedTextAttribute) else {
        return Selection::Unsupported;
    };
    match string_of(&value) {
        Some(text) if text.is_empty() => Selection::Empty,
        Some(text) => Selection::Text(text),
        None => Selection::Unsupported,
    }
}
```

The CF plumbing under it — `Owned` (the +1 reference released on drop, deliberately not `Copy`/`Clone`/`Send`), `element` (the reference as an `AXUIElementRef`), `copy_attribute` (one attribute, owned, `None` on a nonzero status), and `string_of` (the value as a `String` when it is the `CFString` the attribute is documented to hold, a `warn!` and `None` when the app's AX implementation misbehaves) — is this crate's own copy, the same small block `freddie_windows` carries, kept per crate per `docs/platform-apis.md`.

Chrome and Electron web content answer `Unsupported`: their accessibility trees are off until an assistive client announces itself, and nothing here sets the activation flags (`AXEnhancedUserInterface` also changes how Chrome reports window geometry, which the placements sit on, and the extension's content-script route is the planned answer for browser text — a flag poked at Chrome is a workaround that route obviates). The tri-state absorbs it, and the map doubles as the survey of which apps answer — the redacted `Debug` still shows which of the three arms each app is in.

## Order of changes

Two, in order: the extraction (behavior-preserving for `freddie_windows` and mercury), then the selection crate. Nothing in this repo consumes `freddie_selection` when it lands; figaro's doc does.
