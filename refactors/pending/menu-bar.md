# menu bar status

Show the current state in the macOS menu bar: which layer Mercury is in, which app is foregrounded, whatever we want. The menu bar is a status line for the model, so glancing up answers "what will my next key do".

Two stages. Stage 1 is the title: a string that tracks state. Stage 2 is a dropdown: clicking the item opens a menu whose entries execute things in process (dispatch an event, run an effect). The architecture below is shaped so stage 2 is additive, not a rewrite. The one commitment that buys that: the model's render output is a structured value and the crate is a source as well as a sink, from the start, even while stage 1 only uses the title.

## Minimal cut: a present icon and a Quit item

The smallest useful version, and a strict subset of the above: an icon that exists in the menu bar for as long as mercury runs, with a one-item dropdown that quits. No state label, no `menu()` vocabulary, nothing per-layer.

- Create one `NSStatusItem` on the main thread with a fixed title or a template image (an SF Symbol or a short glyph). It stays alive as long as mercury holds it, which is the whole run, so the icon marks that the keyboard grab is active.
- Set an `NSMenu` on it with a single `NSMenuItem`, "Quit", whose action feeds the existing quit path: enqueue the event that a handler turns into `MercuryEffect::Kill`, the same effect `escape` then `q` produces today. AppKit fires the action on the main thread, so the action just sends into `event_tx` like any other source (see "A click is just another event source").

That is one `NSStatusItem`, a fixed title, a one-row `NSMenu`, and one target/action. It is stage 2 with a hardcoded menu instead of a state-derived one, so the `Menu`/`MenuItem`/`Command` types below are not needed until the menu should reflect state. The value beyond aesthetics: a running mercury has grabbed the keyboard, and a visible icon plus a mouse-reachable Quit is a recovery path that does not depend on the keyboard working.

## The label comes from state

`Mercury` grows one method:

```rust
impl Mercury {
    pub fn label(&self) -> String { ... }
}
```

It returns a `String` derived purely from the state tree: the active layer, and for the in-app layer the foregrounded app. Something like `HOME`, `NAV`, `TYPING`, `chrome`. Pure function of `&self`, no I/O, no side effects, trivially testable next to the transition tests. It is the model's half of the contract, exactly parallel to `App::from_name`: the model decides what string represents its state, the renderer decides how to paint it.

The rendering is not the model's job and does not belong in `mercury`. `label` produces text; a separate crate puts it in the menu bar. That split is the whole design: the model stays a pure state machine, the platform code stays quarantined behind a crate boundary.

## What the model produces: a Menu, not just a string

`label` is the title. For the dropdown, the model produces a whole menu, still purely from state:

```rust
pub struct Menu {
    pub title: String,          // == label(), the always-visible text
    pub items: Vec<MenuItem>,   // the dropdown; empty in stage 1
}

pub struct MenuItem {
    pub label: String,
    pub command: Command,       // what clicking it means, as data
    pub enabled: bool,
}

impl Mercury {
    pub fn menu(&self) -> Menu { /* title = self.label(), items from state */ }
}
```

The critical move: a menu item carries a `Command`, an inert enum, not a closure. Clicking does not run code the model handed over; it names an intent the same way a `MercuryEffect` names one. This keeps `menu()` a pure function of `&self` (testable next to the transitions, no captured channels or handles) and keeps the model ignorant of threads and AppKit. `Command` is Mercury's vocabulary of clickable intents; the obvious first move is that a command maps to a `MercuryEvent`, so a click dispatches exactly like a keypress (below). Stage 1 returns `items: vec![]` and nothing downstream changes when they fill in.

## The renderer is its own crate, and it is a source too

The crate, e.g. `freddie_menu_bar`, is sibling to `freddie_app_nav` and `freddie_keyboard`. It is not a string sink; it renders a menu and reports clicks back, so it is both a sink (show this) and a source (a click happened), exactly like `freddie_keyboard::intercept` both swallows and reports keys. Public surface, generic over the command type so the crate stays ignorant of `Mercury`:

```rust
// `on_click` fires (on the main thread) when a dropdown item is chosen,
// handed that item's command. Mirrors freddie_app_nav::watch(cb).
let bar = freddie_menu_bar::MenuBar::new(|command: Command| {
    let _ = event_tx.send(/* command -> MercuryEvent */);
})?;

bar.render(&menu);   // title + items; call again anytime to update
```

Generic over `Command` (the crate never names it), it knows nothing about `Mercury`, `Layer`, or `menu()`, so the FFI stays in one crate the model never depends on and the same renderer can show anything later. The event loop is the glue: after each dispatch it computes `state.menu()` and, when it changed, calls `bar.render(...)`. Recomputing every dispatch is free; the only reason to diff is to skip redundant renders.

`render` mutates the menu bar item directly. No file, no external renderer. The backend is a native `NSStatusItem`, below.

## Rendering model: description in, diff, repaint

`menu()` returns a description of what should be shown; `render` takes that description and makes the menu bar match it. Whether `render` diffs against the last description and patches only what changed, or tears down the `NSMenu` and rebuilds it every time, is the crate's private business and does not matter: the description is the source of truth and a menu bar item plus a handful of dropdown rows is tiny, so full repaint is cheap and patching is a later optimization if it ever shows up in a profile. The model boundary stays the same either way, which is the point of returning a description instead of issuing imperative "add this item" calls. This is the retained-mode/virtual-DOM shape: pure state to description, description reconciled to the platform, reconciliation strategy hidden.

## The backend: a native NSStatusItem

We own the menu bar item rather than routing through SwiftBar. The item is an `NSStatusItem` from `NSStatusBar.systemStatusBar`; the title is `setTitle:` on its `.button()`, and the dropdown is an `NSMenu` set on the item's `.menu`. Each row is an `NSMenuItem` with a title and a target/action; AppKit fires the action, on the main thread, when the row is chosen. Push, no file, no poll, no separate process.

Raw path with `objc2` + `objc2-app-kit`, at the selector level (exact Rust signatures churn across versions and are not compile-checked here, so this is the logic, not a snippet to paste):

- `NSApplication::sharedApplication`, activation policy `Accessory` so there is no dock icon.
- `NSStatusBar::systemStatusBar()`, then `statusItemWithLength:` with `NSVariableStatusItemLength` (`-1.0`).
- Grab `.button()`, `setTitle:` with an `NSString` for the title.
- For the dropdown: build an `NSMenu`, add an `NSMenuItem` per `MenuItem` (title, `setEnabled:`, target/action), set it as the item's `menu`. Rebuild or patch on each `render`.
- `app.run()`.

The click callback needs a target object that implements the action selector and routes each item back to its `Command`. In objc2 that is a small custom `NSObject` subclass (`define_class!`) holding the `on_click` closure and a per-item command, or a single target that maps the sender back to its command. This is the one piece of real objc2 surface beyond setting a title; verify the `define_class!` / target-action shape against the pinned objc2 version when writing it.

`tray-icon` (Tauri's crate) wraps `NSStatusItem` cross-platform and gives both `set_title` and a menu with click events already wired, which removes exactly the target-action objc2 code above. That is the strongest argument for `tray-icon` over raw objc2 once we want a dropdown; weigh it then. Default to raw objc2 for stage 1; reconsider `tray-icon` at stage 2 if the target-action wiring is more than we want to own.

## The main-thread run loop: already paid

`NSStatusItem` is an AppKit UI object, so it must be created and mutated on the main thread, and keeping it live means a run loop turns on the main thread. When this doc was written that was framed as the whole cost, a one-time inversion of main-thread ownership. That inversion has since happened for the app-nav watcher: `crates/mercury/src/main.rs` runs `main_loop.run()` (a `CFRunLoop` via `freddie_main_loop`) on the main thread and runs the tokio event/effect loops on a spawned thread. The keyboard tap is on its own thread with its own run loop. So main-thread ownership already belongs to a run loop, and the status item slots into it rather than forcing a restructure.

The one thing left to confirm, and it is the real open question now: mercury runs `CFRunLoop::run_in_mode`, not `NSApplication::run`. `NSWorkspace` notifications (the app-nav watcher) are delivered fine under a bare `CFRunLoop`, but an `NSStatusItem`'s button and `NSMenu` tracking dispatch `NSEvent`s, which are normally pumped by `NSApplication`'s event loop. Whether the current `CFRunLoop` loop pumps status-item clicks and menu tracking, or whether the item needs `NSApplication` initialized (`NSApp`, an accessory activation policy so no Dock icon, possibly `app.run()` instead of `run_in_mode`) is the thing to verify before writing the FFI. It may be free; it may require swapping the main loop from `CFRunLoop::run_in_mode` to `NSApplication::run`, which `freddie_main_loop` would then own.

The crate hides the main-thread affinity. `MenuBar::new` is created on the main thread (objc2's `MainThreadMarker` enforces this at the type level, which is the reason to prefer objc2 over hand-rolled FFI). `render`, called from the tokio thread after a dispatch, marshals the update to the main queue (`dispatch2`'s main-queue async, or a channel the run loop drains). So the event loop calls `bar.render(&state.menu())` from wherever it runs and the crate gets it onto the main thread.

## A click is just another event source

The click direction is already on the right thread: AppKit fires the action on the main thread, so the `on_click` closure runs there and does the cheapest possible thing, send the item's `Command` (mapped to a `MercuryEvent`) into the same `event_tx` the keyboard and app-nav sources feed. The tokio event loop drains it and dispatches like any other event, producing effects the effect loop performs. So "a menu item executes something in process" reduces to "a click enqueues an event," symmetric to a keypress: no new execution path, no code running inside the model, no locking beyond what the channel already gives. `Refresh` clicked in the dropdown and `r` pressed in the Chrome layer can land on the same `MercuryEvent` and the same handler.

This is why the crate is a source, not just a sink, and why menu items carry inert `Command`s instead of closures: the closure that actually runs is the one `on_click` owns, in the glue layer, and all it does is translate and enqueue. The model never gains a callback, a channel, or a thread.

## Rejected: SwiftBar

SwiftBar was the incumbent (a plugin re-reading a file on an interval). Rejected because we do not want a SwiftBar-launched subprocess resident and polling. The file read was never the cost; the poll interval was, and to catch keypress-driven changes a poll would have to run at roughly keypress latency (~30ms), which is absurd idle work for a status string. SwiftBar's streaming-plugin mode (long-lived process, print a block then a `~~~` line, no interval) would have fixed the latency without polling, but it still means a resident SwiftBar subprocess and a topology question (is mercury the plugin SwiftBar launches, or does a thin plugin stream over IPC). Native `NSStatusItem` is simple enough that neither is worth it: we own the item, push updates directly, and drop SwiftBar, the file, and the poll together.

## Open

- Diffing: `render` only on a changed `menu()`. Decide whether the crate dedupes (compare against last description) or the caller does; either way full repaint on change is fine at this size.
- What an item carries. It should be an event (inert data), not a closure, so `menu()` stays pure. Three shapes: (a) a `MercuryEvent` directly, zero machinery when the action already is an event (foreground an app), but "Refresh" would fake a `KeyEvent`; (b) a new first-class `MercuryEvent::Command(..)` variant plus a trigger wired into the bind derive, so handlers bind to menu commands like keys, no fake input, more work; (c) a distinct `Command` enum mapped to a `MercuryEvent` in the glue. Reusing events means the dropdown and the keybinding hit the same handler. The crate is generic over this type regardless.
- Actions with no state change (Quit is `MercuryEffect::Kill`): keep them on the event path via a handler that returns just the effect, rather than letting an item enqueue an effect directly and splitting dispatch. One path.
- `menu()`'s vocabulary: exact title strings per layer/app, and which items appear per state (e.g. Chrome layer offers `Refresh`, home offers the layer switches).
- Stage 1 scope: ship `title` only (`items: vec![]`), so the objc2 work is just `setTitle:`; defer `NSMenu`, target/action, and the click source to stage 2. The types (`Menu`, `MenuItem`, `Command`, `on_click`) go in now so stage 2 adds no boundary changes.
- Confirm objc2 / objc2-app-kit / dispatch2 versions and the exact `setTitle:`, `NSMenu` target/action (`define_class!`), and main-queue-dispatch call sites against the pinned crates before writing the FFI.
- Where `app.run()` and the tokio threads are wired: mercury's `main` currently owns the tokio runtime on the main thread, so moving tokio to a spawned thread and giving main to AppKit is a `main` restructure, tracked here so it is not a surprise.
