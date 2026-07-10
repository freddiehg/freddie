# the main thread belongs to AppKit

mercury runs its tokio runtime on the main thread. That has to stop. Everything mercury does moves to a spawned thread, and the main thread does nothing but sit in a run loop, so AppKit can deliver its callbacks there.

This is a prerequisite, not a feature. It unblocks foreground-events.md and menu-bar.md, and every Cocoa integration after them. Right now mercury can host none of them.

## What a run loop is, and who owns it

We do not define a run loop. CoreFoundation owns them. Every thread has exactly one, created lazily the first time anyone asks for it, and `CFRunLoop::get_current()` hands back the one belonging to the calling thread. We never construct one and never free one. We add sources to it, and we sit in it.

A run loop is a loop around one blocking receive. Each mode owns a mach port set, a kernel object grouping many ports, and every source contributes a port to it. The thread calls `mach_msg` to receive on the set, which sleeps until a message arrives on any member port, dispatches whichever source it came from, and goes back to sleep. Timers are not special: CoreFoundation arms a `mk_timer` port and puts it in the same set, so a timer firing is another message arriving. (That last detail is from CoreFoundation's implementation, not something we measured.)

Two consequences fall out, and they are the whole doc.

One thread can serve arbitrarily many sources while burning no CPU between them, because it is asleep in a single receive covering all of their ports. One `CFRunLoopRun()` on main was measured serving the `NSWorkspace` port and an unrelated `CFRunLoopTimer` together, callbacks interleaving.

Those callbacks are serialized. Main runs one to completion before starting the next, so two AppKit callbacks never race. That is why AppKit confines itself to one thread and skips locking everywhere. The price is that a slow callback stalls every other source, including the UI.

## What a source is, and who owns it

A source wraps a mach port and binds it to one run loop. `add_source` performs that binding, and the run loop it is given decides which thread services the callback forever after.

Whether you get to choose depends entirely on whether the API hands you the source.

`CGEventTap` hands it over. Inside `CGEventTap::with_enabled` (`core-graphics-0.25.0/src/event.rs:589`) the library does, in order: create the tap, which owns a mach port; wrap that port in a source; add the source to a run loop; enable the tap; then call the closure you passed. The add is this:

```rust
let loop_source = event_tap.mach_port().create_runloop_source(0)?;
CFRunLoop::get_current().add_source(&loop_source, kCFRunLoopCommonModes);
```

`get_current()`, not main. So the tap attaches to whatever thread called `with_enabled`.

`NSWorkspace` does the identical thing internally, with one argument changed: it adds its source to `CFRunLoopGetMain()`, and never returns the source to you. There is no parameter to redirect it and no handle to re-attach it. That one hardcoded argument is the entire reason mercury has to give up main.

Measured, since it is worth not guessing: an observer registered on a spawned thread whose own run loop is running never fires; the same observer fires as soon as main runs a loop, whatever thread registered it; and making the spawned thread the first and only toucher of `NSWorkspace` changes nothing. `NSStatusItem` and the rest of AppKit behave the same. They are main or they are nothing.

## Where the callbacks are defined

Nothing is registered with CoreFoundation by us directly. We pass Rust closures down, and the binding crate installs a C trampoline that calls them.

For the keyboard there are two layers. mercury passes a closure to `freddie_keyboard::intercept` (`mercury/src/main.rs:53`), which is the app-level decision about each key:

```rust
freddie_keyboard::intercept({
    let event_tx = event_tx.clone();
    move |ev| {
        let _ = event_tx.send(MercuryEvent::Key(ev));
        None // swallow; the model dispatches and the effect loop re-emits
    }
})
```

`freddie_keyboard` passes its own closure to `CGEventTap::with_enabled` (`freddie_keyboard/src/sys/macos.rs:225`), which decodes the raw `CGEvent` into a `KeyEvent`, calls the app's `on_key` with it, and turns the answer back into a `CallbackResult`. That is the closure CoreFoundation actually invokes, through core-graphics' trampoline, on the tap thread.

For the foreground watcher there is one layer: an Objective-C block, built with `RcBlock::new`, handed to `addObserverForName:object:queue:usingBlock:`. Foundation invokes it on the main thread. See foreground-events.md.

So "where is the callback defined" has the same answer in both cases. It is a Rust closure captured by a binding crate, invoked from a run loop callback, on whichever thread owns the run loop the source was added to.

## The keyboard tap, end to end

This already works, is unchanged by this doc, and is the concrete example of everything above.

`intercept` spawns a thread (`macos.rs:214`) and calls `CGEventTap::with_enabled` on it. Step three of `with_enabled` runs `CFRunLoop::get_current().add_source(..)`, and because we are on the spawned thread, the tap's port lands in the spawned thread's port set. Step five calls the closure we gave it (`macos.rs:248`):

```rust
|| {
    let _ = ready_tx.send(Ok(CFRunLoop::get_current()));
    CFRunLoop::run_current();
}
```

It hands the run loop back to `intercept` over an `mpsc`, so `Interceptor` can stop it on drop, then blocks that thread in the loop.

Afterwards, a keypress makes the WindowServer send a mach message to the tap's port. The port is in the spawned thread's set, so that thread wakes out of `mach_msg`, CoreFoundation dispatches the source, the tap closure runs, calls mercury's `on_key`, which sends the event into the channel and returns `None`, so the key is dropped. The thread goes back to sleep. All of that happens on the tap thread, synchronously, inside the run loop.

## Why the tap does not move to main

It would be more uniform to put the `CGEventTap` source on main's run loop too, and it is possible without `unsafe`: `CGEventTap::new` (`core-graphics-0.25.0/src/event.rs:570`) takes a `Send + 'static` callback and hands back a tap you hold, rather than the scoped `with_enabled`. Create it, `add_source(CFRunLoop::get_main(), ..)`, enable it, and one thread serves everything.

Do not. A `CGEventTap` sits in the synchronous input path. The WindowServer sends each key to the tap and waits for the verdict before delivering it to anyone, so the callback's return value is the event, and every keystroke on the machine is stalled until it returns. Take too long and macOS disables the tap outright, which is what `CGEventType::TapDisabledByTimeout` is for.

Main-thread callbacks are serialized. Putting the tap there means an `NSStatusItem` click handler that takes 200ms stalls every keystroke in every application for 200ms, and enough of that kills the tap. It couples the machine's input latency to the slowest AppKit handler we ever write.

There is a second reason, specific to mercury: it swallows the keyboard. With the tap on its own thread, a wedged main still leaves keys flowing and `Interceptor::drop` can still release the grab. With the tap on main, a wedged main means no keyboard and no way to fix it, because the thing you would fix it with is the keyboard.

So the split is principled. A notification is late and nobody cares. A tap is a synchronous filter the whole OS blocks on. They want opposite scheduling, and the API that forces you onto main happens to be the one where lateness is harmless.

## How the pieces connect

There is no integration between the run loop and tokio. That is the design. They never share a thread, a scheduler, or a lock. One channel joins them.

```
main thread            tap thread (inside intercept)    worker thread (tokio)
CFRunLoop              CFRunLoop                        block_on(run())
  |                      |                                |
  +- NSWorkspace port    +- CGEventTap source             +- owns Emitter (!Send)
       callback:              callback:                   +- event loop: dispatch -> state
       tx.send(Foreground)    tx.send(Key); drop key      +- effect loop: perform effects
             |                     |                              ^
             +----------+----------+                              |
                        v                                         |
             UnboundedSender<MercuryEvent> ----------------------> rx
```

Three threads, each asleep in its own loop. Two of them are pure event sources feeding one `UnboundedSender`, which is `Send + Sync`, so both hold clones. The worker owns the receiver.

Main never touches state, never dispatches, never blocks. Its callback sends and returns. When `NSStatusItem` arrives it adds a second source to the same loop and does the same thing.

The worker is everything mercury is today: it owns `Mercury`, the `Emitter`, both loops, and the killswitch. It is the only place state is mutated, so there is no shared mutable state and no `Mutex` anywhere.

This is the shape event-loop.md already prescribes, where several sources feed one queue and one event is dispatched per iteration. The main run loop is not a new architecture. It is the missing source.

## The shape of main

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

`intercept` has to be called from inside `run`. It returns the `Emitter`, which is `!Send` (it holds an `Rc`), and the effect loop uses it, so both stay on the thread that created it. Today that thread is main; afterwards it is the worker. The tap is unaffected either way, since it spawns its own thread and adds its source to that thread's loop.

Registering an AppKit observer from the worker is fine. Registration thread is irrelevant, delivery is always on main, and the block can hold a `Send + Sync` sender. This arrangement was built and run as a probe before being written down: tokio on a spawned thread holding an `Rc`, main in a run loop, an observer registered off main, bundle ids arriving in the tokio event loop.

The main thread is the initial thread of the process, the one that entered `main()`. libpthread marks it, `pthread_main_np()` identifies it, `CFRunLoopGetMain()` is its run loop, and nothing can be promoted into it later. `pthread_setname_np`, which `std::thread::Builder::name` calls, renames a thread for debuggers and changes none of that.

## What run_main_loop actually does

```rust
fn run_main_loop(stop: &AtomicBool) {
    const SLICE: Duration = Duration::from_millis(100);
    while !stop.load(Ordering::Acquire) {
        CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, SLICE, false);
    }
}
```

It does not poll for events every 100ms. `run_in_mode` enters the run loop and blocks in `mach_msg` on the port set with a 100ms deadline. A notification arriving at 3ms wakes the thread at 3ms and runs the callback immediately. With `return_after_source_handled = false` it then goes back to blocking and keeps servicing sources until the slice expires, then returns `TimedOut`. Events have no added latency, ever.

The slice governs one thing: how often main surfaces to check `stop`. Shutdown is polled. Events are not.

This is a compromise, and worth calling one. It costs about ten idle wakeups a second, each a syscall return, an atomic load, and a syscall re-entry. The CPU cost is nil; the objection is that a 10Hz heartbeat keeps the core out of deep idle and fights timer coalescing on a daemon meant to sit still all day.

The clean version makes stopping a source, so it is a message in the port set like everything else, and main blocks indefinitely. `core-foundation` exposes `CFRunLoopSource::new` and `from_file_descriptor` but no `signal()` and no `wake_up()`, so that means either calling `CFRunLoopSourceSignal` and `CFRunLoopWakeUp` through `core-foundation-sys` (both `unsafe`), or a pipe-backed `CFFileDescriptor` source woken by writing a byte. Neither is much code. The slice is what we start with.

## Stopping, which is where the bodies are

The worker owns a `Stopper`. Dropping it stops main's run loop, so `run_main_loop` returns and the process exits. That covers a normal return, an early error return (a failed keyboard grab, which today just returns from `main`), and a panic that unwinds. Without it, a worker that dies leaves main asleep forever.

This beats having the worker call `process::exit`, which runs no destructors. One of those destructors is the foreground `Watcher`'s, and it is the one that calls `removeObserver`. Skipping it leaves the notification center holding a block whose closure is gone.

Two things about stopping are not obvious, and both were measured.

`NSRunLoop::run()` is the wrong primitive. It re-enters, so it does not return when the loop is stopped. `CFRunLoop::run_current()` does. `CFRunLoop` is `Send`, and `get_main()` and `stop()` are safe, so none of this needs `unsafe`, and `core-foundation` is already a dependency of `freddie_keyboard`.

`CFRunLoopStop` against a loop that has not started is a no-op. A worker that fails fast, and a failing `intercept` takes microseconds, drops its stopper before main ever enters the loop, and main then blocks forever. That is the same hang, moved. So the stopper is a flag as well as a loop:

```rust
impl Drop for Stopper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release); // seen even if the loop has not started
        self.rl.stop();                           // breaks it out if it has
    }
}
```

Verified to exit cleanly when the stopper is dropped before main enters the loop, and to hang without the flag.

## Who owns the run loop

Not a source library. Two libraries cannot each expose a `run_main_loop()` and both be called, even though their sources would happily share one loop. `freddie_app_nav` registers an observer and documents that main must be running a loop. It does not run one.

Nor mercury. figaro will want the same thing, and so will anything else built on these crates, so the run loop lives in its own crate from the start: `freddie_main_loop`. It depends on `core-foundation` and nothing else.

```rust
/// Pair a main loop with the handle that stops it.
pub fn main_loop() -> (MainLoop, Stopper);

pub struct MainLoop { .. }
impl MainLoop {
    /// Run until stopped. Must be called on the main thread.
    pub fn run(self);
}

/// Send. Dropping it stops the main loop, from any thread.
pub struct Stopper { .. }
```

`run` can check it is really on main without `libc` or objc2, by comparing `CFRunLoop::get_current()` against `CFRunLoop::get_main()`. Calling it anywhere else is a bug that should say so rather than hang.

The crate needs `unsafe_code = "deny"` rather than the workspace's `forbid`, for one operation: `kCFRunLoopDefaultMode` is an extern static (`core-foundation-sys/src/runloop.rs:128`) and reading it is unsafe. `CFRunLoop::get_main`, `get_current`, `stop`, and `run_in_mode` are all safe. Same arrangement as `freddie_app_nav`: its own `[lints]` table copying the workspace's clippy denies, one `#[allow(unsafe_code)]` with a SAFETY comment, and the other crates keep the `forbid`.

## What does not change

The keyboard tap keeps its own thread and its own run loop. `freddie_keyboard::intercept` is untouched.

The event loop, the effect loop, the channels, the killswitch, and the model all move threads without changing shape. `run_effect_loop` stays `!Send` for the same reason it is today.

Nothing about laserbeam, bind, or the model is involved. This is a change to one `main`.

## Open questions

- Whether to build the wake source now rather than accept the 100ms slice.
- Whether the killswitch should drop the `Stopper` rather than call `process::exit`, so the `Watcher` deregisters on the way out. It probably should, which makes `Kill` an ordinary return.
- Whether AppKit requires anything of main beyond a running loop. `NSStatusItem` may want `NSApplication` initialized, which is a bigger commitment than `CFRunLoopRun`. Unmeasured, and menu-bar.md should settle it before assuming this doc is sufficient.
- `addObserverForName:object:queue:usingBlock:` takes an `NSOperationQueue`, and we always pass `None`. If a queue delivers without main running a loop, foreground-events.md would not need any of this. The expectation is that it relocates the callback but not the receiving of the mach message. Unmeasured. It does not change this doc's conclusion, because `NSStatusItem` needs main regardless.
