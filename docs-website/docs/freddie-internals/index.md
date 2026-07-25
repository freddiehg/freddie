---
title: Freddie Internals
sidebar_position: 1
---

# Freddie Internals

The model is a pure function of state and event, and that purity is bought by pushing every impure thing to the edges: threads, operating system handles, and the memory both of those own. This section is about those edges. It is for changing a `freddie_*` crate or adding one, not for writing bindings.

The rules here were mostly learned by getting them wrong. Where a claim has a number attached, that number came from measuring a running `mercury`, and the page says how so you can repeat it.

## The short version

- Three long-lived threads, each with one job, joined only by channels. Nothing shares mutable state, so nothing takes a lock.
- A thread that calls into Cocoa needs an autorelease pool unless it is inside a run loop or about to exit. Mercury's worker thread is neither.
- A `Drop` impl can only release what you were handed. Some operating system calls allocate on your behalf and hand you nothing.
- A balanced reference count is not a reclaimed resource. The only way to know a resource came back is to measure the process.

## In this section

- [Threads](./threads.md) is which threads exist, what each may do, and how work moves between them.
- [Autorelease Pools](./autorelease-pools.md) is who drains them, which threads get one for free, and what happens on the one that does not.
- [Owning Operating System Resources](./owning-os-resources.md) is the ownership newtype, and the two cases it cannot express.
- [Measuring Memory](./measuring-memory.md) is how to tell a leak from a resting size, with the commands.
