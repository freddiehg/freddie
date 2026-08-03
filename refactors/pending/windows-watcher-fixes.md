# the windows watcher reports; it does not decide

Two fixes to `freddie_windows`, plus mercury's consumer half. The governing rule is the synced-state doctrine: a watcher reports what happened with enough identity for the model to judge it, it does not ask the OS whether the report is relevant, and its transport reads are bounded. Figaro's consumer half is `figaro/refactors/pending/sync-fixes.md`, whose change 3 lands after this doc's change 1.

## Change 1: focus reports carry the pid, ungated

The notification callback gates `Focused` reports on `is_frontmost(pid)` — asking the OS a fact every consumer already mirrors — and so reports focus only for the app the OS says is frontmost at callback time. The watcher instead reports every focus change with the pid it is about, and each consumer's model matches it against its mirrored foreground.

The variant, its payload a named struct per the enum form standard; before:

```rust
    /// The focused window changed. `None` when the app that came forward has no focused
    /// window, or its window has no readable id.
    Focused(Option<WindowId>),
```

after:

```rust
    /// The focused window changed in the app with this pid. `window` is `None` when the app
    /// has no focused window, or its window has no readable id.
    Focused(FocusChange),
```

```rust
/// One app's focus change: whose, and to which window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusChange {
    pub pid: Pid,
    pub window: Option<WindowId>,
}
```

The callback arm drops its gate; before:

```rust
    } else if name == kAXFocusedWindowChangedNotification {
        // Only the frontmost app's focused window is what a placement aims at; a background
        // app changing its own focus is not the window the user is looking at.
        if is_frontmost(pid) {
            state.report(WindowChange::Focused(window_id(element)));
        }
    }
```

after:

```rust
    } else if name == kAXFocusedWindowChangedNotification {
        state.report(WindowChange::Focused(FocusChange {
            pid,
            window: window_id(element),
        }));
    }
```

The activation observation reports the same shape, `FocusChange { pid, window: focused_window_id(pid.0) }`, for the app the workspace said activated. `is_frontmost` is deleted; `frontmost_pid` stays, it feeds the snapshot's `focused` seed. `Pid` becomes part of the crate's reported vocabulary, so it is `pub` if it is not already.

### mercury's half

mercury needs the pid mirror the gate used to substitute for. `ForegroundEvent` gains it:

```rust
pub struct ForegroundEvent {
    pub app: App,
    pub pid: Pid,
}
```

mirrored on the root as:

```rust
/// The frontmost app: its pid, which gates the per-app triggers, and what mercury knows
/// about it.
#[derive(Debug)]
pub struct FrontApp {
    pub pid: Pid,
    pub app: ForegroundedApp,
}
```

`foreground: Option<ForegroundedApp>` becomes `Option<FrontApp>`; `record_front_app` builds `FrontApp { pid: ev.pid, app: ForegroundedApp::from_identity(ev.app) }`; the readers respell mechanically (`.as_ref().map(|f| f.app.identity())`, `.as_ref().and_then(|f| f.app.chrome())`); the boot seed's pid comes from `freddie_app_nav`'s frontmost-app snapshot, which reports it beside the bundle id; the `foreground(App)` test constructor gains the pid argument.

The `Focused` row then gates on the mirror through a state-produced trigger, the shape the timer rows already use:

```rust
/// A focus report about this pid. Produced from the mirrored foreground, so the row exists
/// exactly while an app is confirmed, and matches exactly its reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusedFor(pub Pid);
```

with `EventTrigger` matching a `WindowEvent` whose change is `Focused(FocusChange { pid, .. })` with `pid == self.0`, and the root row:

```rust
    |mercury_path| mercury_path.foreground.as_ref().map(|f| FocusedFor(f.pid)) => if_not_invalidated(record_focused),
```

`record_focused` assigns `root.windows.focused` from the report's `window` and nothing else; a background app's report matches no row and dies. Tests: the existing focus-following tests pass with the constructor's added pid; one new test dispatches a `Focused` report for a non-front pid and asserts the model's focused window is unchanged.

## Change 2: the transport reads are bounded

The watcher's callback reads (a frame on move, a focused window on activation) are the sync's transport and stay — but they run under AX's default six-second messaging timeout, on the main run loop everything shares. The crate adopts the one-second bound:

```rust
/// How long any read through an element this watcher holds may take before it is abandoned.
/// The reads run on the main run loop, so a hung app costs this bound there, not the default
/// six seconds. Set per element, so the process-global timeout stays untouched.
const AX_TIMEOUT_SECONDS: f32 = 1.0;
```

applied with `AXUIElementSetMessagingTimeout` to each app element in `observe_app`, each window element in `observe_window`, and the app element `focused_window` creates. A read that misses its bound reports nothing, which every consumer already models as absence.

## Order of changes

Two, independently shippable: change 1 (the crate and mercury together, since the variant's shape change and its consumer respell cannot be split), then change 2 in either order.
