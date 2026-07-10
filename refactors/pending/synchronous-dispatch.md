# synchronous dispatch, and the re-post

mercury swallows every key and re-posts its output through `CGEventPost`. The alternative, which `refactors/past/event-loop.md` prescribed and we did not build, is to dispatch inside the tap callback and return the key as the callback's return value, re-posting nothing.

This is the live decision. It is not a cleanup.

## What we do now

The tap callback sends the event down a channel and returns `None`, so the original key is dropped. The worker thread dispatches, and the effect loop re-synthesizes the output through the `Emitter`, which is `CGEventPost` with a private event source and a tag.

Every keystroke pays this, including plain typing that mercury does not remap. A passthrough key is destroyed and rebuilt.

## What the synchronous model buys

An unremapped key becomes `CallbackResult::Keep` and costs nothing. It is not re-created, does not lose its original timestamp, and is not a synthetic event to anything downstream. Only a real remap replaces the event.

Loop-freedom across processes. Because nothing is re-posted, mercury's output never re-enters the event stream. Today the `EVENT_SOURCE_USER_DATA` tag stops mercury re-eating its own output, but it cannot stop another process feeding that output back. Two remappers with inverse maps would ping-pong. See cgevent-vs-hid.md.

## What it costs, and this is the part that decides it

To return the key output from the tap callback, dispatch has to happen inside that callback, on the tap thread. So the state has to be reachable from the tap thread. There are two ways, and both give up the property mercury currently has, that exactly one thread mutates state and there is no lock anywhere.

Put a `Mutex` around the state and dispatch on the tap thread. This is what event-loop.md prescribed: "a `Mutex` with short critical sections." The worker thread then owns almost nothing.

Or send to the worker and block the tap callback on a reply. Dispatch is microseconds, so it works until the worker hiccups on a page fault, a log write, or the allocator, at which point macOS disables the tap with `TapDisabledByTimeout`. That puts the machine's whole input latency behind a channel round-trip and a scheduler.

So the trade is not "synchronous versus channel". It is one lock-free owner of state, against loop-freedom and no re-post. On the current tap you cannot have both.

## The third option

virtual-hid.md removes the loop hole without dispatching on the tap thread. Seizing the physical device and posting to a separate virtual keyboard makes output structurally not-input, so there is nothing to loop and nothing to tag. It also fixes secure input, which neither of the other two options touches. It is a driver, an entitlement, and a root daemon.

## Where this stands

We accept the re-post, and therefore the cross-process loop hole, because we are the only remapper on this machine. That was never written down until now, which is the reason for this doc.

Nothing has been measured. The re-post's latency, the passthrough cost, and whether a `Mutex` dispatch on the tap thread would actually stay inside the tap's timeout are all guesses.

## Open questions

- Does the re-post cost anything a person can feel, or is this aesthetics? Measure before deciding.
- Would a `Mutex` held for a dispatch stay comfortably inside the tap timeout? Dispatch is microseconds, but the lock is contended by the foreground watcher and any future source.
- Does anything actually break under the re-post besides the theoretical loop? Timestamps, event source, and modifier flags on synthetic events all differ from the originals, and no consumer has complained yet.
- If HID lands, does the synchronous question disappear entirely? It looks like it does: there is no chain to return into, so both backends would be observe-plus-emit.
