---
title: Measuring Memory
sidebar_position: 5
---

# Measuring Memory

Both memory bugs in this codebase were invisible in the source and obvious in the process. Neither was a missed release, so no amount of reading the ownership code would have found either. This page is the loop that does.

## Read the footprint, not the resident size

`ps` reports RSS, which counts shared framework pages your process did not allocate and cannot free. A daemon showing 52MB of RSS had a 16.6MB footprint, and the difference was `__DATA_CONST` and `__TEXT` from AppKit. Footprint is what Activity Monitor calls memory and what you should quote.

```
vmmap -summary $(pgrep -f 'target/debug/mercury daemon')
```

Three parts of that output earn their keep:

- `Physical footprint`, and its peak. A peak equal to the current value means nothing has ever been given back.
- The region table. Region types are diagnostic on their own: `MALLOC_SMALL` is the heap, while 60,000 regions of 16KB under `shared memory` is a window server mapping and not a heap problem at all.
- The `MALLOC ZONE` table, whose allocation count is the single most useful number on the page. It is a live object count, so it falls when things are freed and rises monotonically when they are not.

## Get a rate, not a reading

One number tells you nothing, because a resting size looks exactly like a slow leak until you have two samples and a denominator. Drive a known number of operations, then divide.

```
before: footprint=16.60M  malloc_nodes=27574
after:  footprint=17.70M  malloc_nodes=31592
delta:  footprint=+1.10M  nodes=+4018        over 2000 keys
```

Two allocations per keystroke, and the leak is proven before anyone has guessed at a cause. The denominator matters as much as the delta: the same daemon over a longer window of ordinary use gave 15,919 allocations across 8193 posts, which is 1.94 per post, and agreeing with a 2.00 measured in isolation is what turns a suspicion into a fact.

The log supplies the denominator, since every emitted key writes a `post` record and every dispatch writes one too:

```
grep -c '"pid":PID.*"message":"post"' ~/Library/Logs/mercury/mercury.log
```

## Name the allocation with `heap`

`vmmap` says how much. `heap` says what.

```
heap $(pgrep -f 'target/debug/mercury daemon')
```

It prints live allocations grouped by class. Take it before and after driving your operations and diff the counts. That is what identified the second leak: `CFData` rose 1572 to 3643 and `CFData (Bytes Storage)` rose 1273 to 3343 over 2000 keys, both by roughly 2070, with every other class unmoved. Two objects per post, about 574 bytes, and the class names point straight at what allocates them.

A large `non-object` count is Rust's own allocations, which `heap` cannot name without `MallocStackLogging`.

## Isolate the cause outside the daemon

Once you know what is growing, stop measuring the daemon. Write the smallest program that makes the same call, measure it on itself, and toggle one variable at a time. The daemon has a keyboard tap, 51 observed applications and a log; a twenty-line probe has none of that, and its numbers are clean.

This is how each cause was pinned. One long-lived source against a fresh one per event separated the mapping leak from the flags question. Creating an event against creating and posting one showed that only the post leaks. The same work with and without a pool showed what the fix was worth:

```
create event only, never posted              nodes=   +0 (+0.00/ev)     +0 B/ev
create event + CGEventPost                   nodes=+1001 (+2.00/ev)   +642 B/ev
CGEventPost inside an autorelease pool       nodes=   +0 (+0.00/ev)     +0 B/ev
```

Run the cases in both orders. The first ordering of one benchmark reported a pooled post at six times the cost of an unpooled one, which was backpressure from the preceding batch landing on the second case rather than any cost of the pool.

Defeat the optimiser when timing something small. A `malloc` and `free` whose result is unused compiles to nothing and benchmarks at 0.0ns; assigning through a `volatile` sink gets the real 18.2ns.

## Do not let the measurement mislead you

Two traps cost real time on this codebase.

A short-lived thread cleans up its own autoreleased objects at exit, so measuring poolless Cocoa work on a persistent main thread and concluding the daemon leaks is wrong. Measure on a thread with the same lifetime as the one you are asking about.

And `pgrep -f 'mercury daemon'` matches the shell running your measurement script, because that shell's command line contains the pattern. Two pids arrive where one was expected, every `vmmap` call fails, and a 25-minute sampling run reports zeros. Match the binary path and take the first result.

## What a healthy resting size looks like

So you can recognise one. A debug-build daemon at 16.6MB of footprint: 4.4MB of live heap over 27.9k allocations, of which about 1.5MB is Rust and the rest is Objective-C and CoreFoundation runtime state loaded once, plus 4.8MB of malloc fragmentation, 2.5MB of empty `MALLOC_LARGE`, 1.1MB of `__DATA_DIRTY` and 560KB of thread stacks.

None of that is a leak, and all of it is bigger than the leak was per keystroke. Which is the reason to work in rates: 574 bytes an event is beneath every one of those numbers and still 170MB a day.
