# the selection watcher

Two freddie changes: the per-app observer scaffolding extracted from `freddie_windows`, and `freddie_selection` built on it — a watcher that reports facts, and a read the consumer's effect performers call. The figaro consumer half — the map, the minting, the read effect, the projection — is `figaro/refactors/pending/read-selection.md`, which lands after this doc and after `figaro/refactors/pending/sync-fixes.md`'s pid mirror.

## Change 1 (prefactor): `freddie_ax_observer`

The per-app observer scaffolding moves out of `freddie_windows` into a shared crate, because `freddie_selection` needs machinery that is line-for-line the same: one `AXObserver` per observable app, created at install and at every launch, torn down at termination, its run loop source on the main thread, its `refcon` a boxed registration freed exactly when the observer is released.

`crates/freddie_ax_observer`, carrying the `unsafe_code = "deny"`-with-`#[expect]` lint table (it is an AX boundary crate). What moves in, verbatim except for the genericized seams:

- `ObservableApp` (with `ObservableApp::of`, the UI-service filter). `Pid` is not defined here: it lives in `freddie_windows_types` (`pure-type-crates.md`, landed), and this crate re-exports it.
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

`crates/freddie_selection`, beside `freddie_windows`: the Accessibility calls are raw C, so they live in a freddie platform crate behind safe functions, per `docs/platform-apis.md`. The workspace `members` list gains the crate. Per the vocabulary convention (`pure-type-crates.md`), `Selection` and `SelectionChange` live in a sibling `freddie_selection_types` crate that `freddie_selection` re-exports wholesale; the code below is shown in one listing, with those two types belonging to the types crate.

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

```

The watcher, on the scaffolding:

```rust
The watcher reports facts and answers reads; the sync machinery — minting, `Pending`, `commit` — is the consumer's (`figaro/refactors/pending/read-selection.md`, on `freddie_sync`).

```rust
/// What the watcher can tell you. Facts only: no values, no tokens — the consumer's model
/// requests the value as a read effect when it hears a fact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionChange {
    /// This app's selection changed (or its focus moved between elements): whatever the
    /// consumer knew for this pid is dead.
    Changed(Pid),
    /// The app is gone: remove the entry.
    AppGone(Pid),
}

/// Watch every app's selection. Installed on the main thread, like `freddie_windows::watch`.
pub fn watch(on_change: impl Fn(SelectionChange) + 'static) -> SelectionWatch;
```

Built on `watch_apps`: `on_app` registers `kAXSelectedTextChangedNotification` and `kAXFocusedUIElementChangedNotification` on the app element and reports `Changed(pid)` — which, during the install pass, is also the seed: one burst of `Changed` for every running app, queued as events that dispatch after the consumer's model exists, so the seed path and the steady-state path are the same code and the consumer's map starts empty and honest. The notification callback reports `Changed(pid)` and returns; `on_app_gone` reports `AppGone`. `SelectionWatch` is the `AppWatch` newtype, held for its `Drop`.

The read, called by the consumer's effect performers and nobody else — never by dispatch, never by this watcher:

```rust
/// What `pid`'s focused element answers right now. One synchronous round-trip into the app;
/// callable from any thread, and the app element it creates is its own and per call.
#[must_use]
pub fn current_selection(pid: Pid) -> Selection {
    // SAFETY: `pid` names a process; the element is +1, released with the `Owned`.
    #[expect(unsafe_code)]
    let app = unsafe { AXUIElementCreateApplication(pid.0) };
    let Some(app) = Owned::new(app.cast()) else {
        return Selection::Unsupported;
    };
    let Some(focused) = copy_attribute(element(&app), kAXFocusedUIElementAttribute) else {
        return Selection::Unsupported;
    };
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
