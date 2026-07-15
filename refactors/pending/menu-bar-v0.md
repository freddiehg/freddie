# menu bar v0: implementation

The minimal version: a macOS status item that shows in the menu bar for the whole run, with a one-item dropdown, "Quit", that emits a quit event through the model. This is the step-by-step for it. The design and the rationale are in `menu-bar.md`; this doc is the exact diff. Follow it top to bottom. Commit after each numbered step (the repo commits atomically, per `freddie/CLAUDE.md`).

Two facts drive the whole shape:

- The status item wraps an `NSStatusItem`, and macOS requires that on the main thread with an event loop running. mercury's main thread already runs a loop (`freddie_main_loop`), but it runs a bare `CFRunLoop`, which receives window-server events without ever dispatching them to AppKit. A status-item click is an `NSEvent`; a bare `CFRunLoop` queues it and never calls `sendEvent:`, so the click never reaches the menu. Step 7 replaces the bare `CFRunLoop` with an explicit `NSApplication` event pump (`nextEventMatchingMask` + `sendEvent:`), which is what `[NSApp run]` does, while keeping the existing flag-based stop.
- Quit must kill from any layer, not just home, because it is a recovery path for when the grabbed keyboard is not working. So it is a new event source bound at the ROOT of the model (like `Foregrounded`), not a key binding on the home layer. The existing `q`-from-home quit stays as it is.

The tray lives in a NEW crate, `freddie_menu_bar`, alongside the other platform crates (`freddie_keyboard`, `freddie_app_nav`, `freddie_windows`, `freddie_main_loop`). mercury depends on it.

## Step 1: the new crate `freddie_menu_bar`

Create `crates/freddie_menu_bar/Cargo.toml`:

```toml
[package]
name = "freddie_menu_bar"
description = "A macOS menu-bar status item with a Quit menu, for freddie."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
tray-icon = "0.19"
tracing = "0.1"

[lints]
workspace = true
```

This crate has no `unsafe`, so it keeps the workspace `unsafe_code = "forbid"` (plain `[lints] workspace = true`), unlike `freddie_app_nav`/`freddie_windows`/`freddie_main_loop`, which relax it. tray-icon's API is safe Rust and re-exports muda as `tray_icon::menu`.

Before starting, confirm the current major of tray-icon: `cargo search tray-icon`. If it is not `0.19`, use the latest `0.x` and check that `TrayIconBuilder::with_title` and `tray_icon::menu::{Menu, MenuItem, MenuEvent}` still exist (the version bump between 0.x releases occasionally renames these). Everything below assumes that API.

Create `crates/freddie_menu_bar/src/lib.rs`:

```rust
//! A macOS menu-bar status item with a single Quit entry.
//!
//! [`show`] builds the status item and its one-item menu and registers a callback
//! that fires when Quit is chosen. Call it on the main thread, AFTER NSApp is
//! initialized (see [`freddie_main_loop::init_menu_bar_app`]): tray-icon creates an
//! `NSStatusItem`, which macOS requires on the main thread, and the status item
//! needs an app to live in.
//!
//! The returned [`MenuBar`] owns the status item. Hold it for as long as the icon
//! should be visible; dropping it removes the icon. It is `!Send`, so it stays on
//! the main thread that created it.
//!
//! macOS only.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

/// A live status item. Holding it keeps the icon up; dropping it takes the icon down.
pub struct MenuBar {
    _tray: TrayIcon,
}

/// Shows the menu-bar status item with a single Quit entry. `on_quit` runs, on the
/// main thread, when the user chooses Quit.
///
/// # Errors
///
/// Returns the underlying error if the menu or the status item cannot be created.
pub fn show(
    on_quit: impl Fn() + Send + Sync + 'static,
) -> Result<MenuBar, Box<dyn std::error::Error + Send + Sync>> {
    // The one menu item, and its id so the handler can tell it apart from any future
    // item. `None` is the keyboard accelerator: a status-item menu does not need one.
    let quit = MenuItem::new("Quit", true, None);
    let quit_id = quit.id().clone();

    let menu = Menu::new();
    menu.append(&quit)?;

    // A title rather than an icon for v0: text in the menu bar needs no image asset.
    // `\u{263F}` is ☿, the mercury symbol. Swap for a template image later.
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("\u{263F}")
        .with_tooltip("mercury")
        .build()?;

    // muda delivers menu events through one global handler. It fires on the main
    // thread, during menu tracking, which the NSApp pump (freddie_main_loop) drives.
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == quit_id {
            on_quit();
        }
    }));

    Ok(MenuBar { _tray: tray })
}
```

Notes for the implementer:

- `Menu::append` returns muda's error type and `TrayIconBuilder::build` returns tray-icon's; boxing to `Box<dyn Error>` sidesteps unifying them. Do not change this to a named error enum for v0.
- If `build()` errors about a missing icon on your macOS version (some versions want an icon even with a title), add a 1x1 transparent icon: `.with_icon(tray_icon::Icon::from_rgba(vec![0, 0, 0, 0], 1, 1)?)`. Title-only is expected to work; this is the only contingency.
- `set_event_handler` may require the closure be `Send + Sync`; the signature above already bounds `on_quit` that way, so it compiles regardless.

## Step 2: register the crate in the workspace

`Cargo.toml` (workspace root), `members`:

Before:

```toml
    "crates/freddie_main_loop",
    "crates/freddie_windows",
    "crates/mercury",
]
```

After:

```toml
    "crates/freddie_main_loop",
    "crates/freddie_menu_bar",
    "crates/freddie_windows",
    "crates/mercury",
]
```

## Step 3: the Quit source in the model

`crates/mercury/src/sources.rs`. Add the trigger and its event at the end of the file, mirroring `Foregrounded`/`ForegroundEvent`:

```rust
/// A trigger that matches a quit request, wherever it came from (the menu bar for
/// now). It carries no key: it is a single, layer-independent "quit now".
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quit;

/// A fired quit request.
#[derive(Debug)]
pub struct QuitEvent;

impl EventTrigger for Quit {
    type Event = QuitEvent;
    fn is_matching(&self, _ev: &QuitEvent) -> bool {
        true
    }
}
```

`EventTrigger` is already imported at the top of the file (`use bind::EventTrigger;`).

## Step 4: the unified trigger and event enums

`crates/mercury/src/model.rs`.

Imports, before:

```rust
use crate::{AnyKey, Foregrounded, ForegroundEvent, MercuryEffect};
```

After:

```rust
use crate::{AnyKey, Foregrounded, ForegroundEvent, MercuryEffect, Quit, QuitEvent};
```

`MercuryTrigger`, before:

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyKey(AnyKey),
    Foregrounded(Foregrounded),
}
```

After:

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyKey(AnyKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
}
```

`MercuryEvent`, before:

```rust
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
}
```

After:

```rust
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
}
```

The `derive_more::From` on `MercuryTrigger` gives `From<Quit>` (the derive's check code needs it); the `derive_more::TryInto` with `#[try_into(ref)]` on `MercuryEvent` gives `TryFrom<&MercuryEvent> for &QuitEvent` (dispatch needs it to narrow the event). Both come for free from adding the variants.

## Step 5: the handler and its module

Create `crates/mercury/src/handlers/quit.rs`, mirroring `handlers/foreground.rs`:

```rust
//! The quit source's one handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, QuitEvent};

/// A quit was requested (the menu bar's Quit): kill the program.
///
/// Bound at the root, so it fires from any layer. That is the point: the menu-bar
/// Quit is a recovery path, and it must work whatever layer the model is in, unlike
/// `q`, which quits only from home.
pub(crate) fn on_quit(_ev: &QuitEvent, _node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::Kill]
}
```

Name it `on_quit`, not `quit`: `handlers/home.rs` already has a `quit` (the `q`-from-home handler), and `handlers/mod.rs` glob-re-exports both.

`crates/mercury/src/handlers/mod.rs`.

Before:

```rust
mod app;
mod foreground;
mod home;
mod nav;
mod resize;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use resize::*;
pub(crate) use typing::*;
```

After:

```rust
mod app;
mod foreground;
mod home;
mod nav;
mod quit;
mod resize;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use typing::*;
```

## Step 6: bind Quit at the root, and add the event constructor

`crates/mercury/src/state.rs`.

Imports, before:

```rust
use crate::{AnyKey, App, Foregrounded, ForegroundEvent, MercuryEffect, MercuryEvent, MercuryStruct};
```

After:

```rust
use crate::{
    AnyKey, App, Foregrounded, ForegroundEvent, MercuryEffect, MercuryEvent, MercuryStruct, Quit,
    QuitEvent,
};
```

The root node, before:

```rust
#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(Foregrounded => on_foregrounded)]
pub struct Mercury {
```

After:

```rust
#[derive(Bind, Debug)]
#[node(root)]
#[binds(MercuryStruct)]
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
)]
pub struct Mercury {
```

No layer binds `Quit`, so dispatch bubbles it up to the root every time, from any layer. (It does not collide with typing's `AnyKey` catch-all: `AnyKey`'s event is `KeyEvent`, and a `Quit` event narrows to `&QuitEvent`, not `&KeyEvent`, so `AnyKey` never matches it.)

Add the event constructor next to `key` and `foreground` at the bottom of the file:

```rust
/// A quit-request event (the menu bar's Quit).
#[must_use]
pub const fn quit_event() -> MercuryEvent {
    MercuryEvent::Quit(QuitEvent)
}
```

Named `quit_event`, not `quit`: `state.rs` glob-imports the handlers, so the name `quit` (the home handler) is already in scope here.

## Step 7: export the new items

`crates/mercury/src/lib.rs`.

Before:

```rust
pub use sources::{AnyKey, App, ForegroundEvent, Foregrounded};
pub use state::{
    AppData, AppLayer, ChromeApp, GhosttyApp, HomeLayer, Layer, Mercury, NavLayer, ResizeLayer,
    TypingLayer, foreground, key,
};
```

After:

```rust
pub use sources::{AnyKey, App, ForegroundEvent, Foregrounded, Quit, QuitEvent};
pub use state::{
    AppData, AppLayer, ChromeApp, GhosttyApp, HomeLayer, Layer, Mercury, NavLayer, ResizeLayer,
    TypingLayer, foreground, key, quit_event,
};
```

## Step 8: pump NSApp in `freddie_main_loop`

This is the run-loop change from bare `CFRunLoop` to an `NSApplication` event pump. It is the reason a status-item click reaches the menu.

`crates/freddie_main_loop/Cargo.toml`.

Before:

```toml
[dependencies]
core-foundation = { version = "0.10", features = ["link"] }
```

After:

```toml
[dependencies]
core-foundation = { version = "0.10", features = ["link"] }
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSApplication", "NSResponder", "NSEvent"] }
objc2-foundation = { version = "0.3", features = ["NSDate", "NSRunLoop"] }
```

The version and feature choices match `freddie_app_nav`/`freddie_windows` (objc2 0.6, objc2-app-kit 0.3). objc2 needs the superclass feature, so `NSApplication` pulls `NSResponder`; `NSEventMask` is behind `NSEvent`; `NSDefaultRunLoopMode` is behind `NSRunLoop`. If a build error names a missing type, add the feature that owns it (objc2's docs list which feature each type needs).

`crates/freddie_main_loop/src/lib.rs`.

Imports, before:

```rust
use core_foundation::base::TCFType;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
```

After:

```rust
use core_foundation::base::TCFType;
use core_foundation::runloop::CFRunLoop;
use objc2::MainThreadMarker;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode};
```

`kCFRunLoopDefaultMode` is dropped because `run` no longer uses it; `CFRunLoop` stays because `Stopper` still uses `CFRunLoop::get_main` and `CFRunLoop::stop`, and `is_main_thread` still uses `TCFType`. (Leaving an unused import in would fail the workspace `unused = "deny"` lint.)

Add the app-initialization function. Put it just above `main_loop`:

```rust
/// Initializes `NSApplication` as an accessory (menu-bar) app. Call once, on the
/// main thread, before creating any status item.
///
/// Accessory policy keeps the process out of the Dock and the cmd-tab switcher,
/// which is what a menu-bar-only app wants. `finishLaunching` posts the launch
/// notifications AppKit expects before it delivers events; `[NSApp run]` would do
/// this, but [`MainLoop::run`] pumps the loop itself, so it is done here.
///
/// Separate from [`main_loop`] on purpose: the tests run off the main thread and
/// call `main_loop` to exercise the stop machinery; they must not touch NSApp.
///
/// # Panics
///
/// Panics if called off the main thread.
pub fn init_menu_bar_app() {
    let mtm =
        MainThreadMarker::new().expect("init_menu_bar_app must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    #[allow(unsafe_code)]
    // SAFETY: `finishLaunching` on the shared application, on the main thread.
    unsafe {
        app.finishLaunching();
    }
}
```

`MainLoop::run`, before:

```rust
    pub fn run(self) {
        assert!(
            is_main_thread(),
            "MainLoop::run must be called on the main thread: AppKit delivers only there"
        );
        while !self.stop.load(Ordering::Acquire) {
            // SAFETY: `kCFRunLoopDefaultMode` is an immutable extern static
            // `CFStringRef` that CoreFoundation initializes before `main`.
            #[allow(unsafe_code)]
            let mode = unsafe { kCFRunLoopDefaultMode };
            CFRunLoop::run_in_mode(mode, SLICE, false);
        }
    }
```

After:

```rust
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
                // menu. The SLICE deadline bounds how long a stopped loop takes to notice
                // (the `Stopper` also breaks the run loop, but the deadline is the floor).
                #[allow(unsafe_code)]
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
                    #[allow(unsafe_code)]
                    // SAFETY: dispatching the event we just dequeued, on the main thread.
                    unsafe {
                        app.sendEvent(&event);
                    }
                }
            });
        }
    }
```

`SLICE` (the existing `Duration::from_millis(100)` const) is reused as the per-event deadline; `SLICE.as_secs_f64()` is `0.1`. The `Stopper` is unchanged: its `CFRunLoop::stop` breaks whatever run loop `nextEventMatchingMask` is currently running, and the 100ms deadline is the fallback, exactly as before.

Two objc2 details to confirm as you type, both mechanical (not decisions):

- objc2 maps the selector `nextEventMatchingMask:untilDate:inMode:dequeue:` to the method name `nextEventMatchingMask_untilDate_inMode_dequeue`. If the compiler cannot find it, run `cargo doc -p objc2-app-kit --open` and read the exact name off `NSApplication`.
- `NSDefaultRunLoopMode` is an extern static of type `&'static NSRunLoopMode`; in objc2-foundation it is a safe static. If it is behind an `unsafe` in your version, wrap the read like the current code wraps `kCFRunLoopDefaultMode`.

Also update the crate's module doc: the top-of-file comment describes a `CFRunLoop`; change the sentence that says the main thread parks in `MainLoop::run` and "run a bare `CFRunLoop`" to say it pumps `NSApplication` events, so the doc matches the code. Keep the rest.

## Step 9: wire the tray into mercury

`crates/mercury/Cargo.toml`, `[dependencies]`.

Before:

```toml
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
freddie_windows = { path = "../freddie_windows", version = "0.0.1" }
```

After:

```toml
freddie_main_loop = { path = "../freddie_main_loop", version = "0.0.1" }
freddie_menu_bar = { path = "../freddie_menu_bar", version = "0.0.1" }
freddie_windows = { path = "../freddie_windows", version = "0.0.1" }
```

`crates/mercury/src/main.rs`. Two changes: the event channel moves from `run` up into `main` (the tray's Quit handler runs on the main thread and needs a sender, while the event loop on the worker owns the receiver), and `main` creates the tray.

Import line, before:

```rust
use mercury::{App, Mercury, MercuryEffect, MercuryEvent, Placement, foreground};
```

After:

```rust
use mercury::{App, Mercury, MercuryEffect, MercuryEvent, Placement, foreground, quit_event};
```

`main`, before:

```rust
fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
        eprintln!("windows: {e}");
        error!(error = %e, "window placement unavailable");
    }

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    let worker = std::thread::Builder::new()
        .name("mercury-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(run());
        })
        .expect("spawning the runtime thread");

    main_loop.run(); // services AppKit sources until the worker drops the stopper
    let _ = worker.join();
}
```

After:

```rust
fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());

    // `freddie_windows` reads the screen's visible frame, which is AppKit and so
    // main-thread-bound. Do it here, while we still are one, and cache it.
    if let Err(e) = freddie_windows::init() {
        eprintln!("windows: {e}");
        error!(error = %e, "window placement unavailable");
    }

    // NSApp as an accessory app, before the status item is created and before the
    // loop pumps its events.
    freddie_main_loop::init_menu_bar_app();

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    // The event channel is created here, not in `run`: the menu bar's Quit handler
    // runs on THIS (main) thread and needs a sender, while the event loop on the
    // worker owns the receiver.
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();

    // The status item, on the main thread now that NSApp exists. A Quit click
    // enqueues the same kind of event any source does; the model turns it into
    // `Kill`, which ends the effect loop, releases the keyboard, and drops the
    // stopper. So Quit is the mouse-reachable way out even if the grabbed keyboard
    // is wedged.
    let menu_bar = {
        let event_tx = event_tx.clone();
        freddie_menu_bar::show(move || {
            let _ = event_tx.send(quit_event());
        })
    };
    let menu_bar = match menu_bar {
        Ok(bar) => bar,
        Err(e) => {
            eprintln!("menu bar: {e}");
            error!(error = %e, "could not create the menu bar");
            return;
        }
    };

    let worker = std::thread::Builder::new()
        .name("mercury-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(run(event_tx, event_rx));
        })
        .expect("spawning the runtime thread");

    main_loop.run(); // pumps AppKit events until the worker drops the stopper
    let _ = worker.join();
    drop(menu_bar); // held until the loop returns, so the icon is up for the whole run
}
```

`menu_bar` is `!Send` (`TrayIcon` holds objc pointers); it is created, held, and dropped entirely on the main thread, and is never moved into the worker.

`run`, signature and first lines, before:

```rust
#[allow(clippy::future_not_send)]
async fn run() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();
```

After:

```rust
#[allow(clippy::future_not_send)]
async fn run(
    event_tx: UnboundedSender<MercuryEvent>,
    event_rx: UnboundedReceiver<MercuryEvent>,
) {
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();
```

Nothing else in `run` changes: it still `event_tx.clone()`s for the keyboard and the app-nav watcher, and still seeds the model. `UnboundedSender` and `UnboundedReceiver` are already imported at the top of `main.rs`.

## Step 10: tests

`crates/mercury/tests/transitions.rs`.

Import line, before:

```rust
use mercury::{
    App, Key, KeyEvent, Layer, Mercury, MercuryEffect, MercuryStruct, Placement, PressType,
    foreground, key,
};
```

After:

```rust
use mercury::{
    App, Key, KeyEvent, Layer, Mercury, MercuryEffect, MercuryStruct, Placement, PressType,
    foreground, key, quit_event,
};
```

Add these tests (place them near `home_q_quits`):

```rust
#[test]
fn quit_event_kills_from_home() {
    let mut m = Mercury::default();
    assert_eq!(m.handle(&quit_event()), Some(vec![MercuryEffect::Kill]));
    // No layer change: quit is an effect, not a transition.
    assert!(matches!(m.layer, Layer::Home(_)));
}

#[test]
fn quit_event_kills_from_every_layer() {
    // The menu-bar Quit is a recovery path: it must kill from any layer, not just
    // home. One case per layer.
    for enter in [Key::KeyN, Key::KeyT, Key::KeyR, Key::KeyI] {
        let mut m = Mercury::default();
        let _ = m.handle(&key(enter));
        assert_eq!(
            m.handle(&quit_event()),
            Some(vec![MercuryEffect::Kill]),
            "quit from the layer entered by {enter:?}",
        );
    }
}
```

The second test covers nav, typing, resize, and in-app. Typing is the one that matters most: it binds an `AnyKey` catch-all, and the test proves that catch-all does not swallow the quit event (different event type), so quit still reaches the root binding.

Run `cargo test -p mercury`. The exhaustive-model standard in `freddie/CLAUDE.md` is why quit is asserted from every layer, not just home.

## Step 11: verify end to end, then move this doc

`cargo build` the workspace, then `cargo run -p mercury`. Confirm, using the log (`tail -f ~/Library/Logs/mercury/mercury.log`):

- A ☿ appears in the menu bar and no Dock icon appears (accessory policy).
- Clicking it opens a menu with one item, Quit.
- Choosing Quit produces a dispatch record for the quit event with effects `[Kill]`, then `kill: exiting`, and the process exits cleanly (the keyboard is released, so the terminal is usable again). This is the whole acceptance test: a click reaching the menu is what the Step 8 pump exists to make happen.

If the click does nothing (menu never opens), the pump is not delivering events: re-check that `init_menu_bar_app` runs before `MainLoop::run` and that `run` calls `sendEvent`. If the menu opens but Quit does not exit, the wiring in Step 9 is off (the handler's `event_tx` send, or the `quit_event` binding).

Once it works and the tests pass, move this doc and `menu-bar.md` from `refactors/pending/` to `refactors/past/` (per `freddie/CLAUDE.md`), in the same commit as the last code change or a follow-up.
