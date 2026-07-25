---
title: Owning Operating System Resources
sidebar_position: 4
---

# Owning Operating System Resources

CoreFoundation's rule is that a function with `Create` or `Copy` in its name hands you ownership. The Rust expression of that rule is a newtype whose `Drop` releases, so the release cannot be forgotten when someone later adds a `?` between the call and the end of the function.

```rust
/// A +1 CoreFoundation reference, released when it drops.
///
/// Deliberately not `Copy` and not `Clone`: two of these naming one reference would
/// release it twice.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
```

Most of the time this is the whole story, and it is worth noticing how little of the codebase writes a release by hand. `freddie_keyboard` contains no `CFRelease` at all: `CGEvent` and `CGEventSource` come from `foreign_type!`, which generates `fn drop = |p| CFRelease(p as *mut _)`, so both are released by going out of scope. The release was correct precisely because nobody wrote it.

For which traits to claim and refuse, where `Drop` belongs, and how a C callback reaches its state, read `docs/platform-apis.md` in the repository. This page is about the two cases the ownership newtype cannot express, both of which have cost real memory.

## A newtype can only release what you were handed

`Drop` needs a value to own. Some calls allocate on your behalf and give you nothing:

```c
void CGEventPost(CGEventTapLocation tap, CGEventRef event);
```

No return value and no out-parameter. The two objects it autoreleases are created inside the call and never surfaced, so there is no pointer to wrap and nothing for a `Drop` to release. No amount of ownership discipline reaches them; they need an [autorelease pool](./autorelease-pools.md).

The distinction is exactly the CoreFoundation rule read carefully. `AXUIElementCopyAttributeValue` has `Copy` in its name and hands you a +1 pointer through an out-parameter, so `Owned` can take it. `CGEventPost` transfers nothing, so the objects it allocates were never yours, and the rule that makes the first case work is the same rule that says so.

## A balanced reference count is not a reclaimed resource

This one is worse, because the code reads as correct at every level.

`CGEventSourceCreate(kCGEventSourceStatePrivate)` returns a CoreFoundation object, and releasing it does everything CoreFoundation promises. It also asks the window server to stand up private event state and maps about 16KB for it, and that mapping is not owned by the object, not counted by its reference count, and not returned by any API. The object's lifetime and the mapping's lifetime are unrelated.

While mercury built a source per emitted key, the count of those mappings tracked the count of posted keys one to one: 61,444 posts against 61,458 regions of 16KB each, which is how a keyboard remapper reached a gigabyte of resident memory in five hours. The heap was 45MB of that; the rest was mappings whose objects had all been released properly.

So: for anything you create per event, do not conclude from the ownership rules that it is free. Measure the process. [Measuring Memory](./measuring-memory.md) is how.

There is usually a hint in the timing if you look for it. Creating a private source measured 20.2µs against 6.6µs for reusing one, and that 13.6µs is a round trip to the window server doing setup work. It got read as a CPU cost worth 0.14% of a core rather than as evidence that something was being allocated on every call.

## Create once what you can create once

The fix for the source was to create one per thread that builds events rather than one per event, which is also the general shape. A handle that costs a round trip to a system service, and that the API lets you keep, should be kept.

Where keeping it changes behaviour, fix the behaviour rather than paying per event. Posting through a source mutates its flag state, and reading those birth flags back is what made a long-lived source unsafe. So the emitter stopped reading them:

```rust
let intrinsic = intrinsic_flags(code);
event.set_flags(to_cg(flags) | intrinsic);
```

What goes on the wire is now a function of the key and the caller's modifiers, and of nothing the source is holding, which makes the lifetime of the source irrelevant to correctness.

Two details from doing that, both of which cost a measurement to learn. The bits a key carries of its own accord belong to the keycode, not to a `Key` variant, because the keypad has no variant and arrives as `Key::Raw`, so a table of variants drops the bit for exactly the keys named after it. And the four arrows carry `NumericPad` while `Home`, `End`, `PageUp` and `PageDown` do not, which you find by building an event for every keycode from 0 to 127 and reading the flags back rather than by reasoning about which keys feel like a navigation cluster.

## Register once, deregister once

A resource with a registration has two sides, and the second is easy to lose. An `AXObserver` per app, a notification per window, an `NSWorkspace` observer: each needs the thing that removes it to be as reachable as the thing that added it, which usually means the registration lives in a `Drop` next to what it registered.

Removal order matters when a callback dereferences state. `AppObserver::drop` removes the run loop source before releasing the observer and before the boxed `refcon` its callbacks read is freed, so no notification can arrive holding a stale pointer.

The cost of getting this wrong is bounded rather than unbounded, which is why it hides. Observing every GUI application on the machine the way `freddie_windows` does, 114 apps and 25 windows, costs 0.73MB in total, about 7KB per app. That is small enough that a registration leaked per app is invisible for a long time and still wrong.
