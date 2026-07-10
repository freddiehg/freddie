# synchronous dispatch, and the re-post

mercury swallows every key and re-posts its output through `CGEventPost`. The alternative, which `refactors/past/event-loop.md` prescribed and we did not build, is to dispatch inside the tap callback and return the key as the callback's return value, re-posting nothing.

Decided: no. We keep the re-post, and virtual-hid.md is where this actually goes.

The reason is not that the synchronous model is bad. It is that the channel model mercury already has is the shape HID wants, and the synchronous model is a detour HID would undo. virtual-hid.md is explicit: both backends sit behind observe-plus-emit, and "the one thing that would leak is CGEventTap's trick of deciding in the callback and returning the event down the chain. HID has no chain to return into." Seize the device and every key the user sees is one you emitted. There is no pass, no `Keep`, no `Replace`, and nothing to optimize by not re-posting, because re-posting is the only mechanism there is.

So the passthrough fast path, the snapshot of the accumulated trigger set, and the opt-out-of-capture policy below all evaporate under HID. They are recorded because they are good ideas for the tap, and because we are on the tap until the driver exists.

The rest of this doc is the record of what we accept in the meantime, and why.

## What we do now

The tap callback sends the event down a channel and returns `None`, so the original key is dropped. The worker thread dispatches, and the effect loop re-synthesizes the output through the `Emitter`, which is `CGEventPost` with a private event source and a tag.

Every keystroke pays this, including plain typing that mercury does not remap. A passthrough key is destroyed and rebuilt.

## What the synchronous model buys

The tap chain gives four outcomes, and three of them cost nothing:

- pass, which is `Keep`
- a one-to-one remap, which is `Replace`
- a swallow, which is `Drop`
- a one-to-many chord, which returns one event and posts the rest through the tap proxy

Only the chord posts. An alternative keyboard layout is overwhelmingly one-to-one remaps, so a heavy remapper is the case this helps most, not least. Today mercury posts every key, including the one-to-one ones, and including keys it passes through unchanged.

Loop-freedom, but only partly. Nothing is re-posted for the first three outcomes, so their output never re-enters the event stream. Chords still post, and cgevent-vs-hid.md is explicit that "any async decision or one-to-many output" forces the re-post. mercury's `refresh` turns one `r` into `cmd down, r down, r up, cmd up`, so the loop hole survives the synchronous model for exactly the bindings that emit chords. Loop-freedom for real only comes from HID.

## What it costs, and this is the part that decides it

To return the key output from the tap callback, dispatch has to happen inside that callback, on the tap thread. So the state has to be reachable from the tap thread. There are two ways, and both give up the property mercury currently has, that exactly one thread mutates state and there is no lock anywhere.

Put a `Mutex` around the state and dispatch on the tap thread. This is what event-loop.md prescribed: "a `Mutex` with short critical sections." The worker thread then owns almost nothing.

Or send to the worker and block the tap callback on a reply. Dispatch is microseconds, so it works until the worker hiccups on a page fault, a log write, or the allocator, at which point macOS disables the tap with `TapDisabledByTimeout`. That puts the machine's whole input latency behind a channel round-trip and a scheduler.

So the trade is not "synchronous versus channel". It is one lock-free owner of state, against loop-freedom and no re-post. On the current tap you cannot have both.

## The third option

virtual-hid.md removes the loop hole without dispatching on the tap thread. Seizing the physical device and posting to a separate virtual keyboard makes output structurally not-input, so there is nothing to loop and nothing to tag. It also fixes secure input, which neither of the other two options touches. It is a driver, an entitlement, and a root daemon.

## You cannot know without dispatching

Whether a key passes through is a function of the state. In `Home`, `d` is swallowed; in `Typing`, `d` passes. Same key, opposite answers. So there is no shortcut that decides a key without asking the model, which is the whole reason the synchronous model needs state reachable from the tap thread.

There is a way out, and it is the reason to keep the idea alive. Let each state declare what it does with keys it does not bind, as an opt-out of capture rather than a binding: `Home` swallows, `Typing` passes. Combine that with `bind::accumulate`, which already computes the set of triggers the active state cares about and which nothing consumes, and the tap callback can hold an immutable snapshot of (bound set, default policy). Then an unbound key is decided by a hash lookup, with no state access and no lock, and only bound keys need dispatch. `handlers-as-values.md` extends this: a handler that is a value rather than a function can have a statically known output, so a pure remap resolves in the callback too.

This would also delete a wart. `TypingLayer` expresses "pass everything" as an `AnyKey` catch-all, which shadows the layer-level `escape` binding, which is why typing has to re-bind `escape`. A passthrough policy makes the catch-all unnecessary.

The hazard is staleness. The snapshot is published by the worker after it dispatches. Press `t` to enter typing and then `d` quickly enough, and `d` reaches the tap before the worker has processed `t`, so `d` is decided against `Home`'s policy and swallowed. Today that cannot happen, because both keys traverse one channel in order. The fix is to take the fast path only when no events are in flight, which is an atomic counter, and it needs to be designed in rather than discovered.

## Where this stands

We accept the re-post, and therefore the cross-process loop hole.

Not because we are the only remapper. Karabiner-Elements is running on this machine, with its DriverKit virtual HID device, driven by voicemode's `karabiner.edn`. But it cannot loop with us: it seizes the physical keyboard at the IOKit HID level and reads HID reports, not `CGEvent`s, and posts to its virtual device. Our tap is at `Session`, downstream, and our output is posted into `Session`, downstream of it. Karabiner never sees what we emit. Every key mercury sees has already been through Karabiner.

A loop needs two peers at the same level that both re-post. Whether one exists is answerable: `CGGetEventTapList` enumerates every tap with its owning process, tap point, and whether it filters or only listens. It is not in the `core-graphics` crate.

Nothing has been measured. The re-post's latency, the passthrough cost, and whether a `Mutex` dispatch on the tap thread would actually stay inside the tap's timeout are all guesses.

## Open questions

- Does the re-post cost anything a person can feel, or is this aesthetics? Measure before deciding.
- Is the snapshot fast path worth its staleness race, and is the in-flight counter enough to close it?
- What does a passthrough policy look like on a node: a trait method, or an attribute the derive reads?
- Would a `Mutex` held for a dispatch stay comfortably inside the tap timeout? Dispatch is microseconds, but the lock is contended by the foreground watcher and any future source.
- Does anything actually break under the re-post besides the theoretical loop? Timestamps, event source, and modifier flags on synthetic events all differ from the originals, and no consumer has complained yet.
- If HID lands, does the synchronous question disappear entirely? It looks like it does: there is no chain to return into, so both backends would be observe-plus-emit.
