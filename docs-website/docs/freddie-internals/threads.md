---
title: Threads
sidebar_position: 2
---

# Threads

Three threads run for the life of the process, each asleep in its own loop, joined only by a channel. A handful more are spawned for single slow operations and exit immediately.

## The main thread is a doorman

It runs the platform run loop and nothing else. `AppKit` delivers callbacks only while the main thread is inside a run loop, so that is where it stays, and `freddie_main_loop::main_loop` is what puts it there.

Main-thread callbacks are serialized, so a slow one stalls every other source. That is why a callback's whole body is a channel send:

```rust
let _app_watcher = freddie_app_nav::watch({
    let event_tx = event_tx.clone();
    move |bundle_id| {
        let _ = event_tx.send(foreground(App::from_bundle_id(bundle_id)));
    }
});
```

Anything main-thread-only is created here and held here: the status item, the overlay panel, the `NSWorkspace` and screen observers, and every `AXObserver`, whose run loop sources are added to this thread's loop. Work going the other way is drained on each wake, in `main_loop.run`'s closure, so a title change or an overlay message is applied by the thread allowed to apply it.

## The tap thread carries the keyboard

`freddie_keyboard::intercept` spawns it and it runs a `CFRunLoop` of its own for the `CGEventTap`. It has always been off main, which is why the keyboard keeps working whatever main is doing. Its callback does the same thing every other callback does: turn the event into a portable `KeyEvent`, send it, return.

## The worker thread owns the state

Named `mercury-runtime`, it runs a current-thread tokio runtime and both loops: the event loop, which dispatches and mutates state, and the effect loop, which performs what dispatch produced.

Two properties come from it being one thread rather than from any marker or lock:

- It is the only place state is mutated, so there is no shared mutable state and no `Mutex`.
- It is the only consumer of the effect channel, so effects are performed in the order dispatch produced them, and a modifier reaches the operating system before the key carrying its flag.

It is also the thread with the least help from the platform. It has no run loop and it never exits before the process does, which is what [Autorelease Pools](./autorelease-pools.md) is about.

## Move data with a channel, not a lock

The preferred way to move data between threads is a channel whose sender is `Send` and cloneable while the receiver stays pinned to one thread. Sending an event to the thread that owns the state beats reaching into that state across a lock. When a design reaches for `Arc<Mutex<_>>`, the first question is what channel would carry that data instead.

The channel's flavour follows its receiver. Events and effects use tokio's `unbounded_channel`, because the worker awaits them inside the runtime. Titles and overlay messages use a `freddie_main_loop::WakingSender`, a std channel under a waker, because the receiving end is the main thread sitting in `AppKit`, not a task in the runtime. Sending wakes the loop, so a title change is applied at once rather than at the next unrelated event.

## Spawn a thread for anything slow

The effect loop must not block, so an effect that costs more than microseconds gets a detached thread and is never joined:

```rust
fn set_frame(windows: Option<&WindowSink>, target: WindowFrame) {
    let Some(windows) = windows.cloned() else { return; };
    std::thread::spawn(move || match windows.set_frame(target) {
        Ok(()) => debug!(?target, "set the window's frame"),
        Err(e) => warn!(?target, error = %e, "set frame failed"),
    });
}
```

Setting a window frame takes tens of milliseconds, which is long enough to delay a key the effect loop is about to emit. Foregrounding an app and writing the pasteboard get the same treatment. A detached thread also cannot hold up the exit path the way `spawn_blocking` would.

These threads are the easy case for cleanup: they exit, and exiting is itself a cleanup event.

## Do not poll

An idle system costs nothing. With no work it is asleep, not surfacing on a timer to check. Work arrives by waking whatever the consumer is parked on, either the channel it is receiving from or the operating system wait it is blocked in when that wait cannot be selected on. A loop that wakes every N milliseconds to look for work is polling even when it is dressed as a timeout or a run-loop slice, and that N is a latency floor paid in power for the life of the process.

You can see this in the log. With nobody typing, a daemon dispatches nothing and its footprint does not move.

## Threads and `Send`

Do not add a `Send` bound to describe an intention. The guarantees above come from one channel with one consumer, and no marker was ever enforcing them.

Some types are honestly not `Send`, and that is fine as long as it is deliberate. `Emitter` holds a `CGEventSource`, which comes from `foreign_type!` and wraps a `NonNull`, so `Emitter` is not `Send` and the futures owning it are not either. Both of mercury's say so:

```rust
#[expect(clippy::future_not_send)]
async fn run_effect_loop(
```

That is accurate rather than unfortunate: posting mutates a source, so a source belongs to the thread that posts through it. Note that a wrapped pointer is not automatically thread-hostile, and the reverse trap exists too: `core-foundation` declares `unsafe impl Send for CFRunLoop`, so a `CFRunLoop` crosses threads freely and `Interceptor` stays `Send` while `Emitter` does not.
