# foreground events

Knowing which app is frontmost and getting an event when it changes, fed into the runner as an input alongside keys. The counterpart to app-foregrounding.md, which triggers the change this observes.

`freddie_app_nav` does this today by polling `osascript`. This doc is about replacing that with the `NSWorkspace` observer, and about the one thing that makes it awkward: the observer only works if the process's main thread is pumping a run loop, and mercury's main thread belongs to tokio.

## What we need

- The current frontmost app at startup.
- An event each time the frontmost app changes, carrying enough to identify it.
- That identity mapped onto the app's `App` set (Chrome, Ghostty, Zed, Other for mercury) and sent into the event channel, where `Foregrounded` dispatch records it. The mapping is the consumer's, so the crate hands up a string and nothing else.

## Before: the osascript poll

`crates/freddie_app_nav/src/lib.rs` spawns a thread that shells out every 250ms and diffs the answer against the last one.

```rust
fn frontmost() -> Option<String> {
    const SCRIPT: &str = "tell application \"System Events\" to name of first application process whose frontmost is true";
    let output = Command::new("osascript").arg("-e").arg(SCRIPT).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
}

// in Watcher::spawn
let handle = thread::spawn(move || {
    let mut poller = Poller::new();
    while running_thread.load(Ordering::Relaxed) {
        if let Some(app) = poller.observe(query()) {
            on_change(&app);
        }
        responsive_sleep(poll_interval, &running_thread);
    }
});
```

Three things are wrong with it. It spawns a subprocess every tick, which is absurd for reading one string. It lags by up to the poll interval. And it hands up a display name rather than a stable identifier, which is why `App::from_name` matches `"Google Chrome"` but `"ghostty"` and `"zed"` in lowercase: those are System Events process names, and they are not what the apps call themselves. It also trips an Automation permission prompt for System Events, which the `NSWorkspace` route does not need.

## After: the NSWorkspace observer

`NSWorkspace` gives the frontmost app at startup and a notification on every change, with `bundleIdentifier` as the identity. This compiles and runs today against `objc2 = "0.6"`, `objc2-app-kit = "0.3"`, `objc2-foundation = "0.3"`, `block2 = "0.6"`.

```rust
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSRunLoop};

/// The app a `didActivateApplication` notification is about.
fn activated_app(notif: &NSNotification) -> Option<Retained<NSRunningApplication>> {
    let info = notif.userInfo()?;
    let key = unsafe { NSWorkspaceApplicationKey }; // extern static
    info.objectForKey(key)?.downcast::<NSRunningApplication>().ok()
}

let ws = NSWorkspace::sharedWorkspace();

// Startup: the app that is already frontmost.
let current = ws.frontmostApplication().and_then(|a| a.bundleIdentifier());

// Changes: one callback per activation, delivered on the main thread.
let block = RcBlock::new(|notif: core::ptr::NonNull<NSNotification>| {
    let notif = unsafe { notif.as_ref() };
    if let Some(app) = activated_app(notif) {
        on_change(&app.bundleIdentifier().unwrap().to_string());
    }
});
let _token = unsafe {
    ws.notificationCenter().addObserverForName_object_queue_usingBlock(
        Some(NSWorkspaceDidActivateApplicationNotification),
        None, // any object
        None, // no queue: deliver on the posting thread
        &block,
    )
};

NSRunLoop::currentRunLoop().run(); // must be the MAIN thread
```

No `Poller`, no interval, no `responsive_sleep`, no subprocess, no diffing. The notification only fires on a real change, so the whole change-detection apparatus goes away. Switching apps produced exactly this:

```
startup frontmost: Some("dev.zed.Zed")
NOTIFIED: Some("com.google.Chrome")
NOTIFIED: Some("com.mitchellh.ghostty")
NOTIFIED: Some("dev.zed.Zed")
```

## What was measured, and how

Each of these was checked with a probe rather than assumed, because several of them are not what you would guess.

The read path needs no `unsafe`. `NSWorkspace::sharedWorkspace()`, `frontmostApplication()`, `bundleIdentifier()`, `localizedName()`, and `notificationCenter()` all compile under `#![forbid(unsafe_code)]`.

The observer needs exactly three `unsafe` operations, and no more. `addObserverForName_object_queue_usingBlock` is an `unsafe fn`; `NSWorkspaceDidActivateApplicationNotification` and `NSWorkspaceApplicationKey` are extern statics whose use is unsafe. Dereferencing the `NonNull<NSNotification>` the block receives is a fourth if you count it separately.

The main thread must pump a run loop, and that is the expensive constraint. An observer registered on a spawned thread whose own run loop is running never fires, if main is not pumping. The same observer registered on a spawned thread does fire when main pumps a run loop, and the block is always invoked on the main thread. So registration thread is irrelevant and the main run loop is mandatory.

The obvious escape, that Foundation might attach lazily to the run loop of whichever thread first touches `NSWorkspace`, does not work. With main never calling `NSWorkspace` at all, and the watcher thread being both the first toucher and the only thread pumping a run loop, the observer fired zero times across two real app switches. Foundation installs the port on `CFRunLoopGetMain()`, not on the caller's run loop.

Why any of this is so: the kernel only delivers a mach message to a port. The main thread is a userland fact that Foundation records at process init (`pthread_main_np()`), and `CFRunLoopGetMain()` is that thread's run loop. `NSWorkspace` registers its port as a source on that run loop. A message sitting in the port does nothing until someone pumps that run loop, at which point Foundation dequeues it and posts the `NSNotification` synchronously on the pumping thread. So the block does not run because the notification is never posted, not because it is posted and routed elsewhere.

Polling `frontmostApplication()` is not a workaround. It reads a cache that the workspace notification machinery refreshes, not the live window server. With no run loop pumping it returns the app that was frontmost at process start, forever. This was checked twice: polling from a spawned thread while main pumped a run loop, and polling from the main thread with no run loop at all. In both cases `open -a` returned exit status 0 for three real app switches and the polled value never moved off `dev.zed.Zed`. Any design that swaps `osascript` for an `NSWorkspace` poll is silently broken.

The bundle identifiers are stable and are what we want: `com.google.Chrome`, `com.mitchellh.ghostty`, `dev.zed.Zed`. Note that `localizedName` reports `"Zed"` where System Events reports `"zed"`, which is the discrepancy that made the current name table have to be lowercase.

Dropping the observer token does not stop the observation. Registering, then dropping both the returned `Retained` token and the `RcBlock`, then switching apps twice, still invoked the block twice. Only `center.removeObserver(observer)` stopped it, after which nothing fired. Cocoa's contract is that `removeObserver:` must be called before the returned observer is deallocated, so dropping the token and letting the block keep running is precisely the unsound case: the block was invoked after the Rust value that owned it was gone. A callback closing over an `UnboundedSender` in that state is a use-after-free waiting to happen.

## What this does to the Watcher

`Watcher` currently promises that dropping it stops the source, and under the poll it delivers: `Drop` flips an `AtomicBool` and joins the thread. Under `NSWorkspace` that promise breaks, because the notification center holds the observer regardless of what Rust thinks it owns. The new `Watcher` has to hold the token and call `removeObserver` in `Drop`, and stopping the source means stopping the run loop (`CFRunLoop::stop`) rather than setting a flag the loop reads on its next tick. The `AtomicBool` and `responsive_sleep` both disappear with the poll.

## What it costs mercury

mercury's main thread is `#[tokio::main(flavor = "current_thread")]`, so it never pumps a run loop and would never receive a notification. Making this work means main runs `NSRunLoop::currentRunLoop().run()` and the tokio runtime moves to a spawned thread.

That is not a free move, because of `Emitter`. It is `!Send` (it holds an `Rc`), it is created by `freddie_keyboard::intercept` on the calling thread, and the effect loop uses it. So `intercept` has to be called on whichever thread ends up owning the effect loop, and the `Emitter` must never cross a thread. Today `main` calls `intercept` and hands the `Emitter` to `run_effect_loop` on the same thread via `join!`, which is why the effect loop is `!Send`. Under the new shape, the spawned tokio thread calls `intercept`, keeps the `Emitter`, and runs both loops; main does nothing but pump the run loop and hold the process open.

The keyboard tap is unaffected: `intercept` already spawns its own thread with its own `CFRunLoop` for the tap, and that keeps working regardless of what main does.

## The lint

The workspace sets `unsafe_code = "forbid"`. `forbid` cannot be relaxed by an inner `#![allow(unsafe_code)]`; that is a hard error, not a warning. `freddie_keyboard` gets to do FFI at zero `unsafe` only because `core-graphics` wraps everything in safe Rust, and objc2 does not do that for these three items.

So one of two things has to happen before this can be written. Either `freddie_app_nav` drops `[lints] workspace = true` for its own table that keeps every clippy deny but softens `unsafe_code` to `deny`, leaving the rest of the workspace on `forbid`; or the workspace lint itself softens to `deny` and each crate re-forbids. The first keeps the blast radius to one crate.

## Open questions

- Where the bundle-id to `App` map lives (with the app's bindings, as the name table does now), and how figaro overrides it.
- Whether "window changed within the same app" matters, or only "app changed". `didActivateApplication` only fires for the latter.
- Debouncing rapid app switches so a flurry of foreground events does not thrash the layer. Less pressing than under the poll, since the notification fires once per real activation.
- Whether the `Watcher` handle can still stop the source cleanly. Stopping means removing the observer and stopping the run loop (`CFRunLoop::stop`), rather than flipping an `AtomicBool` the poll loop reads.
- Whether the `watch(interval, callback) -> Watcher` signature survives. The interval argument becomes meaningless, so it should go, and `DEFAULT_POLL_INTERVAL` with it.
- Whether mercury wants a `NSWorkspaceDidDeactivateApplicationNotification` counterpart, or the activation event is enough.
