---
title: Architecture
sidebar_position: 1
---

# Architecture

Every freddie app has the same shape, which will be familiar to anyone acquainted with the [Elm architecture](https://guide.elm-lang.org/architecture/). A program maintains some state and receives a stream of events. Those events are dispatched, which may result in a handler getting called. Which handler runs depends on the program state. A handler can mutate the state and return effects. Those effects are then performed.

The regular-program part is the setup: subscribe to streams of events, turn those into an enum, call `let effects = state.handle(event).unwrap_or_default()`, and perform each effect.

The freddie part is everything that happens inside `state.handle(event)`. `handle` is a pure state transformer — state and event in, updated state and effects out — which is what makes it easy to test.

## In this section

- [The Event Loop](./the-event-loop.md)
- [The Data Model](./the-data-model.md)
- [Dispatch and Precedence](./dispatch-and-precedence.md)
- [Typed Paths](./typed-paths.md)
- [Virtual Fields](./virtual-fields.md)
- [The Crates](./the-crates.md)
