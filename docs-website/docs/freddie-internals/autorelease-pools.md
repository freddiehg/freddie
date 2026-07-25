---
title: Autorelease Pools
sidebar_position: 3
---

# Autorelease Pools

An autorelease pool is not a memory pool. It is a per-thread stack of pending `release` calls.

When an Objective-C or CoreFoundation function returns an object it does not want you to own, it cannot release it before returning, so it calls `autorelease` on it: add this to the current thread's pool, and release it whenever that pool is drained. Draining the pool is what actually sends all those deferred releases. Nothing else does.

The consequence that matters here is that a thread only has a pool if something put one there.

## Which threads get one for free

Two things drain a pool without you asking.

A `CFRunLoop` pushes a pool at the top of each iteration and drains it at the bottom. So the main thread and the tap thread are both covered, and autoreleased objects on them live until the end of the current turn of the loop.

Thread exit drains too. `libobjc` registers a thread-local destructor that pops the remaining pool page when a thread dies, so a thread that does some Cocoa work and exits cleans up after itself. Measured with 20,000 autoreleased `NSString`s and no pool anywhere:

```
on the MAIN thread, no pool, thread keeps running: nodes  +20050
on a SPAWNED thread, no pool, after it exits:      nodes      +5
```

That is why the detached threads mercury spawns per placement, per copy and per foregrounding need nothing: they exit, and exiting is the drain.

## The thread that gets neither

Mercury's worker thread has no run loop and never exits before the process does. Anything autoreleased on it is added to a pool that nobody will ever pop, so it is leaked for the life of the run.

`CGEventPost` autoreleases two CoreFoundation objects per call, about 574 bytes, most likely the serialized event it ships to the window server. The worker thread posts every key mercury emits, which is every key you press, so before this was fixed the daemon leaked 574 bytes per keystroke. `heap` named it: over 2000 keys, `CFData` went from 1572 to 3643 and `CFData (Bytes Storage)` from 1273 to 3343, both up by roughly 2070, with every other class flat.

The fix is a pool around the work:

```rust
fn post(&self, key: Key, press: PressType, flags: ModifierFlags) -> Result<(), EmitError> {
    autoreleasepool(|_pool| {
        let event = keyboard_event(&self.source, key, press, flags)?;
        self.tag.stamp(&event);
        event.post(CGEventTapLocation::Session);
        Ok(())
    })
}
```

## The rule

If you call into Cocoa or CoreFoundation from a thread that is neither sitting in a run loop nor about to exit, wrap the work in `objc2::rc::autoreleasepool`.

Put the pool in the crate that makes the call, not in the consumer that drives it. `CGEventPost` is what autoreleases, `Emitter::post` is the only place it happens, so `freddie_keyboard` owns the pool. Pushing that responsibility outward would ship a public `emit` that leaks unless every caller knows to wrap it, and would need the identical fix again in every program using the crate. A `freddie_*` crate's decisions are justified by that crate's own constraints, never by what a consumer happens to do.

Wrap the call that allocates, not the one that looks expensive. Creating an event allocates nothing that needs draining, measured at 0 allocations over 500 iterations; only the post does. A pool around `keyboard_event` alone would look right and fix nothing.

## Do not batch

The instinct to amortise the pool over many operations is wrong twice. Per-operation is as fast or faster, and batching holds garbage. Two autoreleased objects per simulated post, 400,000 posts per case:

```
  pool every      1 posts      62.3 ns per post        0.6 KB of garbage held
  pool every      8 posts      70.0 ns per post        4.5 KB of garbage held
  pool every     64 posts      66.4 ns per post       35.9 KB of garbage held
  pool every    512 posts      71.5 ns per post      287.0 KB of garbage held
  pool every   1024 posts      71.2 ns per post      574.0 KB of garbage held
  pool every   4096 posts      70.8 ns per post     2296.0 KB of garbage held
```

There is no fixed cost to amortise. Pushing and popping an empty pool is 5.2ns and the release work is proportional to the number of objects, not the number of pools, so batching saves one 5.2ns pair and pays for it by draining cold memory instead of objects still in cache. Against a real `CGEventPost` at 5,000 to 10,000ns, the whole question is noise.

## Why it is a closure and not a guard

The first thing you reach for is an RAII guard in a local, and objc2 deliberately does not offer one. Pools form a per-thread stack and must be popped in reverse order of creation. A guard sitting in a variable can be dropped early, moved into a struct, or forgotten, any of which pops out of order and corrupts the stack. From objc2's own source:

```
// - The pools are guaranteed to be dropped in the reverse order they were
//   created (since you can't possibly "interleave" closures).
//
//   This would not work if we e.g. allowed users to create pools on the
//   stack, since they could then safely control the drop order.
```

So this is the one release in the codebase that is a scope rather than a value. It is still drop-based; it just owns the pool instead of the objects, because the pool is the only handle the API gives you to their lifetime.

`objc2::rc::autoreleasepool` is a safe function, so reaching for it does not cost a crate its `unsafe_code = "forbid"`.
