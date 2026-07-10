# foreground events

Knowing which app is frontmost and getting an event when it changes, fed into the runner as an input alongside keys.

`freddie_app_nav` polls `osascript` for this today. We are replacing that with the `NSWorkspace` `didActivateApplication` observer. The motivation is not latency, which is fine. The poll is ugly: it spawns a subprocess every tick to read one string, it keys apps by display name, and it re-derives "did it change" by diffing something the OS already knows. Every piece of that machinery exists only because we are not asking the system the question it is willing to answer.

The prerequisite is done. `NSWorkspace` delivers only while the main thread is inside a run loop, and mercury's main now is: see `refactors/past/main-thread.md` for why, and `freddie_main_loop` for the code. Nothing about the run loop, the `Stopper`, or the exit path is this doc's problem any more.

## What we are building

`freddie_app_nav::watch` becomes an observer. The callback fires once per real activation, carrying a bundle identifier.

```rust
pub fn watch<F>(on_change: F) -> Watcher
where F: Fn(&str) + Send + 'static
{
    let block = RcBlock::new(move |notif: NonNull<NSNotification>| {
        // SAFETY: Foundation hands the block a valid, retained notification.
        let notif = unsafe { notif.as_ref() };
        if let Some(bundle_id) = activated_bundle_id(notif) {
            on_change(&bundle_id);
        }
    });

    // SAFETY: the block is Send; Foundation invokes it on the main thread; the
    // observer is removed in Watcher::drop before the block is dropped.
    let token = unsafe {
        NSWorkspace::sharedWorkspace().notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None, // any sender
                None, // no queue: deliver on the posting thread, which is main
                &block,
            )
    };
    Watcher { token, _block: block }
}

/// The bundle id of the frontmost app right now. Seeds the initial state.
pub fn frontmost() -> Option<String> {
    NSWorkspace::sharedWorkspace().frontmostApplication()?
        .bundleIdentifier().map(|id| id.to_string())
}
```

The crate registers a source and nothing else. It does not run a loop; `freddie_main_loop` does, and mercury calls it.

Deleted: `Poller`, `responsive_sleep`, `DEFAULT_POLL_INTERVAL`, the `AtomicBool`, the `JoinHandle`, and the `osascript` `frontmost`. The crate stops owning a thread, and the diffing goes with the poll, since the notification only fires on a real change.

The identity becomes the bundle id (`com.google.Chrome`, `com.mitchellh.ghostty`, `dev.zed.Zed`) rather than the display name. The mapping stays the consumer's: mercury's `App::from_name` becomes `App::from_bundle_id`. That deletes the wart where the name table had to spell `ghostty` and `zed` in lowercase, because that is what System Events calls them while the apps call themselves `Ghostty` and `Zed`.

Three semantic changes come with it. `watch` loses its interval argument. `watch` no longer reports the app that is frontmost at registration, because the notification only fires on activation, so the caller seeds with `frontmost()`. And the callback tightens from `FnMut` to `Fn`, because `RcBlock` wants a shared callable.

## What mercury changes

`run` registers the watcher instead of the poll, and seeds the initial app itself:

```rust
if let Some(id) = freddie_app_nav::frontmost() {
    let _ = event_tx.send(foreground(App::from_bundle_id(&id)));
}
let _watcher = freddie_app_nav::watch({
    let event_tx = event_tx.clone();
    move |bundle_id| {
        let _ = event_tx.send(foreground(App::from_bundle_id(bundle_id)));
    }
});
```

Registering from the worker thread is fine: registration thread is irrelevant, delivery is always on main. The block does one `send` and returns, because main-thread callbacks are serialized and a slow one stalls every other source.

`App::from_name` and `App::launch_name` become `App::from_bundle_id` and `App::bundle_id`. The `Foreground` effect keeps shelling out to `open` for now, so it needs the bundle id as its argument, which `open -b` takes.

The `Watcher` is `!Send`, so it lives on the worker thread that registered it, which is where `run` already keeps it.

## What is specific to NSWorkspace, and measured

Most of this is not what you would guess, so none of it is assumed.

- Registration thread is irrelevant. An observer registered on a spawned thread fires, and the block is always invoked on main.
- The thread running the loop is not irrelevant. An observer whose own thread runs a run loop never fires if main is not in one.
- Foundation does not attach lazily to whoever touches `NSWorkspace` first. With main never calling it, and the watcher thread both first-touching and solely running a loop, it fired zero times. The port goes on `CFRunLoopGetMain()`.
- Polling `frontmostApplication()` is not a workaround. It reads a cache the notification machinery refreshes, so with no run loop running it returns the app that was frontmost at process start, forever. Checked from a spawned thread with main running a loop, and from main with no loop; three real app switches, value never moved. This is why `frontmost()` is only good for seeding.
- Dropping the observer token does not deregister it. The center kept calling the block after both the token and the `RcBlock` were dropped. Only `removeObserver` stopped it.

## The Watcher

`Watcher` promises that dropping it stops the source, and under the poll it delivers: `Drop` flips an `AtomicBool` and joins the thread. That promise has to be kept a different way, because the notification center holds the observer regardless of what Rust thinks it owns. `Watcher` holds the token and the block, and `Drop` calls `removeObserver`. Leaking it instead leaves the callback live and callable after whatever it closed over is gone, which for a callback holding an `UnboundedSender` is a use-after-free. Holding an `RcBlock` and a `Retained` makes `Watcher` `!Send`.

mercury's exit path is already ready for that. `Kill` ends the effect loop rather than calling `process::exit`, so `run` returns, the `Stopper` drops, and destructors run all the way out. A `Watcher` held on the worker thread will deregister on the way.

## The lint

The workspace sets `unsafe_code = "forbid"`, and `forbid` cannot be relaxed from inside the crate. The observer needs four unsafe operations: `addObserverForName_object_queue_usingBlock` and `removeObserver` are `unsafe fn`s, `NSWorkspaceDidActivateApplicationNotification` and `NSWorkspaceApplicationKey` are extern statics, and the block's `NonNull<NSNotification>` has to be dereferenced. Everything else, including the whole read path, compiles under `forbid`.

`freddie_app_nav` drops `[lints] workspace = true` for its own table that denies `unsafe_code` and copies the workspace's clippy denies, with `#[allow(unsafe_code)]` and a SAFETY comment at each site. `freddie_main_loop` already does exactly this for one extern static, so this is the second crate to opt out. A third would argue for softening the workspace lint instead of accumulating per-crate tables.

## The tests get worse

`cargo test` never puts the main thread in a run loop, so the observer cannot fire and cannot be unit-tested. Today's seven tests cover the `Poller` diff exhaustively and drive the watcher loop against a scripted source, which works only because the poll is generic over its query function. All of that goes with the poll. What survives is `open_args` and a smoke test that `frontmost()` returns something bundle-id-shaped, so verification of the interesting half moves out of the suite.

`freddie_main_loop` hit the same wall and settled for asserting on the stop flag rather than running a loop. Here the equivalent would be a test binary that gives main to a run loop and drives real app switches, at the cost of a test that steals focus and cannot run headless.

## Open questions

- Is a test binary that runs a main loop worth building to keep the observer under test?
- Does `Watcher` keep a consuming `stop()` alongside `Drop`, now that stopping is just deregistering?
- Where the bundle-id to `App` map lives (with the app's bindings, as the name table does now), and how figaro overrides it.
- Whether "window changed within the same app" matters, or only "app changed". `didActivateApplication` only fires for the latter.
- Whether mercury wants a `NSWorkspaceDidDeactivateApplicationNotification` counterpart, or activation is enough.
- `addObserverForName:object:queue:usingBlock:` takes an `NSOperationQueue`, and we always pass `None`. If a queue delivered without main running a loop, none of the main-thread work would have been needed for this doc. It was needed for `NSStatusItem` regardless, so this is now idle curiosity rather than a risk.
