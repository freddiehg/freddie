# the main thread belongs to AppKit

mercury runs its tokio runtime on the main thread. That has to stop. Everything mercury does moves to a spawned thread, and the main thread does nothing but sit in a run loop, so AppKit can deliver its callbacks there.

This is a prerequisite, not a feature. It unblocks foreground-events.md and menu-bar.md, and every other Cocoa integration after them. Right now mercury can host none of them.

## Why AppKit needs the main thread

An AppKit source registers itself with the main thread's run loop, and a run loop only delivers while some thread is inside it. `CFRunLoopRun()` is the loop:

```
loop {
    deadline = earliest_timer_deadline()
    msg = mach_msg_receive(port_set, timeout: deadline)  // one blocking syscall
    dispatch(msg)                                        // run the callback, return, repeat
}
```

The main thread is the initial thread of the process, the one that entered `main()`. libpthread marks it, `pthread_main_np()` identifies it, and `CFRunLoopGetMain()` is its run loop. Nothing can become it later.

So the rule is not "run loops must be on main." It is that a source lives on exactly one run loop, and only the thread owning that loop can service it. Whether you get to choose the loop depends on whether the API hands you the source.

`CGEventTap` hands it over, which is why the keyboard already works off main. `core-graphics` does this for us:

```rust
let loop_source = event_tap.mach_port().create_runloop_source(0)?;
CFRunLoop::get_current().add_source(&loop_source, kCFRunLoopCommonModes);
```

`get_current()`, so `freddie_keyboard` spawns a thread, adds the source to that thread's run loop, and runs it there. mercury therefore already has a run loop on a spawned thread today.

`NSWorkspace` does not hand it over. It creates its source internally and adds it to `CFRunLoopGetMain()` with no way to redirect it. Measured: an observer registered on a spawned thread whose own run loop is running never fires; the same observer fires when main runs a loop, whatever thread registered it; and making the spawned thread the first and only toucher of `NSWorkspace` changes nothing. `NSStatusItem` and the rest of AppKit are the same. They are main or they are nothing.

## One run loop, many sources

Giving main to a run loop does not spend it on one thing. A run loop multiplexes: it blocks on a whole port set at once and dispatches whichever source fires. One `CFRunLoopRun()` on main was measured serving the `NSWorkspace` port and an unrelated `CFRunLoopTimer` together, their callbacks interleaving. `NSWorkspace`, an `NSStatusItem`, and a timer all register with the same loop, and main sleeps on all of them.

The callbacks are serialized. Main runs one to completion before starting the next, so two AppKit callbacks never race, which is the reason AppKit confines itself to one thread and skips locking. The price is that a slow callback stalls every other source, including the UI.

That fixes the rule for main-thread callbacks: they do nothing. The foreground observer's block sends a bundle id into a channel and returns. A menu-bar click sends an event and returns. Microseconds. The work happens on the tokio thread.

## The shape

```rust
fn main() {
    let log_path = init_tracing();
    println!("mercury: logging to {}", log_path.display());

    let stop = Arc::new(AtomicBool::new(false));
    let stopper = Stopper::new(Arc::clone(&stop)); // Send; holds CFRunLoop::get_main()

    std::thread::spawn(move || {
        let _stopper = stopper; // dropping it stops main, however we leave
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(run()); // everything main() does today
    });

    run_main_loop(&stop); // main does nothing else until stopped
}
```

`run` is the current body of `main`, moved wholesale. `#[tokio::main]` goes away in favour of building the runtime by hand, because the attribute would put it back on main.

`intercept` has to be called from inside `run`. It returns the `Emitter`, which is `!Send` (it holds an `Rc`), and the effect loop uses it, so both must stay on the thread that created it. Today that thread is main; afterwards it is the tokio thread. The keyboard tap is unaffected either way: it already spawns its own thread with its own run loop.

Registering an AppKit observer from the tokio thread is fine. Registration thread is irrelevant, delivery is always on main, and the callback can hold a `Send + Sync` `UnboundedSender` into the tokio runtime. This whole arrangement was built and run as a probe before being written down: tokio on a spawned thread holding an `Rc`, main in a run loop, an observer registered off main, and bundle ids arriving in the tokio event loop.

## Stopping, which is where the bodies are

The worker thread owns a `Stopper`. Dropping it stops main's run loop, so `run_main_loop` returns and the process exits. That covers a normal return, an early error return (a failed keyboard grab, which today just returns from `main`), and a panic that unwinds. Without it, a worker that dies leaves main asleep in the run loop forever.

This is strictly better than having the worker call `process::exit`, which runs no destructors. One of those destructors is the foreground `Watcher`'s, and it is the one that calls `removeObserver`. Skipping it leaves the notification center holding a block whose closure is gone.

Two things about stopping are not obvious, and both were measured.

`NSRunLoop::run()` is the wrong primitive. It re-enters, so it does not return when the loop is stopped. `CFRunLoop::run_current()` does. `CFRunLoop` is `Send`, and `get_main()` and `stop()` are safe, so none of this needs `unsafe`, and `core-foundation` is already a dependency of `freddie_keyboard`.

`CFRunLoopStop` against a loop that has not started is a no-op. A worker that fails fast, and a failing `intercept` takes microseconds, drops its stopper before main ever enters the loop, and then main blocks forever. That is the same hang, moved. So the stopper is a flag as well as a loop, and main runs in bounded slices:

```rust
impl Drop for Stopper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release); // seen even if the loop has not started
        self.rl.stop();                           // breaks it out if it has
    }
}

fn run_main_loop(stop: &AtomicBool) {
    const SLICE: Duration = Duration::from_millis(100);
    while !stop.load(Ordering::Acquire) {
        CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, SLICE, false);
    }
}
```

The slice bounds shutdown latency, not event latency: sources are still serviced the instant they fire, inside `run_in_mode`. Verified to exit cleanly when the stopper is dropped before main enters the loop, and to hang without the flag.

## Who owns the run loop

Not a library. Two libraries cannot each expose a `run_main_loop()` and both be called, even though their sources would happily share one loop. `freddie_app_nav` registers an observer and documents that main must be running a loop. mercury runs it.

If a second consumer appears, the `Stopper` and `run_main_loop` move into one small crate that both depend on, rather than into either of them. They are about thirty lines and depend only on `core-foundation`.

## What does not change

The keyboard tap keeps its own thread and its own run loop. `freddie_keyboard::intercept` is untouched.

The event loop, the effect loop, the channels, the killswitch, and the model all move threads without changing shape. `run_effect_loop` stays `!Send` for the same reason it is today.

Nothing about laserbeam, bind, or the model is involved. This is a change to one `main`.

## Open questions

- Where `Stopper` and `run_main_loop` live before a second consumer exists: `mercury/src/main.rs`, or a crate from the start.
- Whether the killswitch should drop the `Stopper` rather than call `process::exit`, so the `Watcher` deregisters on the way out. It probably should, which makes `Kill` an ordinary return rather than an exit.
- Whether `AppKit` requires anything of main beyond a running loop. `NSStatusItem` may want `NSApplication` initialized, which is a bigger commitment than `CFRunLoopRun`. Unmeasured, and menu-bar.md should settle it before assuming this doc is sufficient.
- `addObserverForName:object:queue:usingBlock:` takes an `NSOperationQueue`, and we always pass `None`. If a queue delivers without main running a loop, foreground-events.md would not need any of this. The expectation is that it relocates the callback but not the receiving of the mach message, so main is still required. Unmeasured. It does not change this doc's conclusion, because `NSStatusItem` needs main regardless, but it would change the urgency.
