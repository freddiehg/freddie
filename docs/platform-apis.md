# Wrapping an operating system API

What the `freddie_*` crates do when they hold something the OS gave them. A type that owns an OS resource asks for as little as it can and gives back exactly what it took.

## Ask for the least

Claim the traits the type actually needs, not the ones that would be convenient. `Element` is `Send`, because one is handed to the thread performing a placement; it is not `Sync`, because two threads never reach the same one. Every `unsafe impl` is a claim you have to defend, and the narrower claim is the easier one.

`!Send` is a feature when the resource is thread-bound. An `NSPanel` belongs to the main thread, so a handle whose `Drop` touches one is `!Send` and cannot be moved somewhere that `Drop` would silently do nothing. `Watcher` and `MenuBar` are the same. Say it with `PhantomData<*const ()>` rather than a runtime check.

When a worker needs to reach a thread-bound thing, split the type rather than loosening it: an owner that stays put, and a `Send` handle that carries a message to it. `Watcher` and `WindowSink` are that pair.

`RefCell`, `Mutex`, and `RwLock` each move a borrow check to runtime. Take one only when something genuinely crosses a thread or is aliased. `WatcherState` keeps its apps in a `RefCell` because every writer is on the main thread; the window table is a `Mutex` because a placement reads it from another. Neither is there to make a lifetime problem go away.

A lock is not a substitute for a field being in the wrong struct. If a callback cannot reach a field, moving it to something the callback can reach is the fix; wrapping it in a `Mutex` is not.

Between `Mutex` and `RwLock`, prefer `Mutex` unless readers actually contend. A window opening and a key being pressed are both rare, so concurrent readers have nothing to win.

## Give back what you took

Put `Drop` on a newtype around the one resource, not on the struct that happens to hold several. `Owned` wraps a single +1 CoreFoundation reference and releases it, so `freddie_windows` has exactly one `CFRelease` and no path that can skip it. A `Drop` on a large struct releasing five things in order is where the order becomes something you have to know, and a reordered field becomes a bug.

With the resources wrapped, the outer type usually needs no `Drop` at all. `Watcher` has none: dropping it drops the map, which drops each `AppObserver`, which removes its run loop source and releases its observer.

A missing `Drop` is only correct when what you hold already undoes itself. `TrayIcon` removes its icon when dropped; `Retained<NSPanel>` does not take the panel off screen, because AppKit's window list holds its own reference. Check which kind you have.

Registering with the OS returns something to hold. A notification observer, a run loop source, a global event handler: each has a deregistration, and it belongs in the `Drop` of whatever owns the registration. `Observation` holds its notification centre alongside its token, because deregistering needs the centre that registered.

Where drop order matters despite all this, say so at the field. Fields drop in declaration order, and `Watcher` declares its observations first so they stop before the state their callbacks write into is torn down.

Prefer a resource that survives reuse to one rebuilt each time, and make the difference explicit in the API. Hiding a panel and dropping it are different operations, because one will be shown again and the other will not.

## Talking to C

A C callback has no environment, so an API that calls you back takes a `refcon` — an opaque pointer it hands straight back. That is the closure capture, done by hand. The box holding it must outlive every notification that names it, which means releasing the registration before dropping the box, and it must be passed everywhere the callback dereferences it. A callback registered with a null `refcon` on one notification and a real one on another is a null dereference waiting for that notification to fire.

An untyped out-pointer is a type hole; close it with a trait rather than a convention. `AxAttribute` pairs an attribute's name, its `AXValueType`, and the Rust type that type means, so asking for a size and writing it into a point does not compile. Anything that takes a `*mut c_void` and a separate tag has this shape.

CoreFoundation's naming is the ownership rule: a function with `Create` or `Copy` in its name hands you a reference you must release, and one with `Get` does not. Wrap the first kind the moment you get it.

A panic must not cross an FFI boundary — unwinding through a C frame is undefined. A callback should be total: no `unwrap` on anything the OS controls, and no assumption that a value it hands you is well formed.

Private symbols are acceptable when they are the only route, and should be isolated so their absence costs one feature rather than the crate. `_AXUIElementGetWindow` is the only way from an `AXUIElement` to a `CGWindowID`; a window whose id cannot be read is skipped, and everything else goes on working.

## The main thread

`AppKit` and the Accessibility observers deliver on the main thread, from its run loop, and main-thread callbacks are serialized. A callback that does real work stalls every other source, so it should hand the work elsewhere and return. Sending on a channel is the intended body.

Work that must happen on main, from a thread that is not main, has two routes. `DispatchQueue::main().exec_async` runs a block promptly, because the main queue is drained from inside the run loop, but the block must be `'static` and `Send`, so it cannot carry a thread-bound value and has to find one already there. A channel drained in `freddie_main_loop`'s `on_wake` can carry anything, but waits for the current slice to end.

Reading OS state is something you do while constructing, before `main_loop.run`. After that every fact arrives as an event. See the seed rule in `CLAUDE.md` and `refactors/past/seed-at-construction.md`.
