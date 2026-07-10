# foreground events

Knowing which app is frontmost and getting an event when it changes, fed into the runner as an input alongside keys.

`freddie_app_nav` polls `osascript` for this today. We are replacing that with the `NSWorkspace` `didActivateApplication` observer. The motivation is not latency, which is fine. The poll is ugly: it spawns a subprocess every tick to read one string, it keys apps by display name, and it re-derives "did it change" by diffing something the OS already knows. Every piece of that machinery exists only because we are not asking the system the question it is willing to answer.

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

/// Hands the calling thread (which must be main) to the run loop, forever.
pub fn run_main_loop() -> ! {
    loop { NSRunLoop::currentRunLoop().run(); }
}
```

`run_main_loop` lives in this crate so mercury never takes an objc2 dependency of its own.

Deleted: `Poller`, `responsive_sleep`, `DEFAULT_POLL_INTERVAL`, the `AtomicBool`, the `JoinHandle`, and the `osascript` `frontmost`. The crate stops owning a thread, and the diffing goes with the poll, since the notification only fires on a real change.

The identity becomes the bundle id (`com.google.Chrome`, `com.mitchellh.ghostty`, `dev.zed.Zed`) rather than the display name. The mapping stays the consumer's: mercury's `App::from_name` becomes `App::from_bundle_id`. That deletes the wart where the name table had to spell `ghostty` and `zed` in lowercase, because that is what System Events calls them while the apps call themselves `Ghostty` and `Zed`.

Three semantic changes come with it. `watch` loses its interval argument. `watch` no longer reports the app that is frontmost at registration, because the notification only fires on activation, so the caller seeds with `frontmost()`. And the callback tightens from `FnMut` to `Fn`, because `RcBlock` wants a shared callable.

## The constraint: main must pump the run loop

`NSWorkspace` installs its mach port on the main thread's run loop. A message in a port does nothing until some thread is inside the loop that owns it, so nothing is delivered unless main is pumping. The block does not fail to run because the notification is routed elsewhere; the notification is never posted at all.

Measured, because most of it is not what you would guess:

- Registration thread is irrelevant. An observer registered on a spawned thread fires, and the block is always invoked on main.
- Pumping thread is not irrelevant. An observer whose own thread runs a run loop never fires if main is not pumping.
- Foundation does not attach lazily to whoever touches `NSWorkspace` first. With main never calling it, and the watcher thread both first-touching and solely pumping, it fired zero times. The port goes on `CFRunLoopGetMain()`.
- Polling `frontmostApplication()` is not a workaround. It reads a cache the notification machinery refreshes, so without a pumping run loop it returns the app that was frontmost at process start, forever. Checked from a spawned thread with main pumping, and from main with no run loop; three real app switches, value never moved.
- Dropping the observer token does not deregister it. The center kept calling the block after both the token and the `RcBlock` were dropped. Only `removeObserver` stopped it.

## What it costs mercury

mercury's main thread is `#[tokio::main(flavor = "current_thread")]`, so it is already inside an event loop, and a thread cannot be inside two. Main takes the run loop and everything else moves to a spawned thread:

```rust
fn main() {
    let log_path = init_tracing();
    println!("mercury: logging to {}", log_path.display());

    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(run()); // everything main() does today
    });

    freddie_app_nav::run_main_loop(); // main does nothing else, ever
}
```

`intercept` has to be called from inside `run`, on the tokio thread, because it returns the `Emitter`, which is `!Send` (it holds an `Rc`) and is used by the effect loop. The observer can be registered from that same thread; delivery is on main regardless, and its callback pushes into the same `Send + Sync` `UnboundedSender` the keyboard uses. This shape was built and run before being proposed.

The keyboard tap is unaffected. `intercept` already spawns its own thread with its own `CFRunLoop`, and does not care what main does.

One bug the restructure introduces: failing to grab the keyboard currently prints and returns from `main`, exiting the process. Under the new shape that `return` happens on the tokio thread while main sits in `run_main_loop` forever, so mercury hangs. The tokio thread has to `process::exit`, or signal main to stop the run loop.

## The Watcher

`Watcher` promises that dropping it stops the source, and under the poll it delivers: `Drop` flips an `AtomicBool` and joins the thread. That promise has to be kept a different way, because the notification center holds the observer regardless of what Rust thinks it owns. `Watcher` holds the token and the block, and `Drop` calls `removeObserver`. Leaking it instead leaves the callback live and callable after whatever it closed over is gone, which for a callback holding an `UnboundedSender` is a use-after-free. Holding an `RcBlock` and a `Retained` makes `Watcher` `!Send`.

## The lint

The workspace sets `unsafe_code = "forbid"`, and `forbid` cannot be relaxed from inside the crate. The observer needs four unsafe operations: `addObserverForName_object_queue_usingBlock` and `removeObserver` are `unsafe fn`s, `NSWorkspaceDidActivateApplicationNotification` and `NSWorkspaceApplicationKey` are extern statics, and the block's `NonNull<NSNotification>` has to be dereferenced. Everything else, including the whole read path, compiles under `forbid`.

`freddie_app_nav` drops `[lints] workspace = true` for its own table that denies `unsafe_code` and copies the workspace's clippy denies, with `#[allow(unsafe_code)]` and a SAFETY comment at each site. The other crates keep the `forbid`. The cost is duplicating the clippy list in one manifest; the alternative, softening the workspace to `deny`, is one line but gives up the guarantee everywhere.

## The tests get worse

`cargo test` does not pump a main run loop, so the observer cannot fire and cannot be unit-tested. Today's seven tests cover the `Poller` diff exhaustively and drive the watcher loop against a scripted source, which works only because the poll is generic over its query function. All of that goes with the poll. What survives is `open_args` and a smoke test that `frontmost()` returns something bundle-id-shaped, so verification of the interesting half moves out of the suite. A test binary that pumps its own run loop and drives real app switches would keep it under test, at the cost of a test that steals focus and cannot run headless.

## Open questions

- Does `watch` seed the initial app itself, or does the caller call `frontmost()`?
- Is a run-loop-pumping test binary worth building to keep the observer under test?
- Does `Watcher` keep a consuming `stop()` alongside `Drop`, now that stopping is just deregistering?
- Where the bundle-id to `App` map lives (with the app's bindings, as the name table does now), and how figaro overrides it.
- Whether "window changed within the same app" matters, or only "app changed". `didActivateApplication` only fires for the latter.
- Whether mercury wants a `NSWorkspaceDidDeactivateApplicationNotification` counterpart, or activation is enough.
