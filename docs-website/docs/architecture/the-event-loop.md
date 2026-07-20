---
title: The Event Loop
sidebar_position: 2
---

# The Event Loop

TODO: the setup side of a `freddie` program — subscribing the sources, the select that feeds one queue, one event dispatched per iteration, and the effect loop that follows.

## Prior art

`freddie`'s event loop follows two existing systems. [`isograph`](https://github.com/isographlabs/isograph)'s language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. [`barnum`](https://github.com/barnum-circus/barnum) goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. `freddie`'s difference from `barnum` is that its handlers mutate state directly during dispatch, where `barnum`'s only return a value the engine writes back.

## `mercury`'s setup

`mercury` uses `clap` to parse commands. The main subcommand is `mercury start`, which calls a hidden internal command, `mercury daemon`, which does the following:

- Takes the single-instance lock, so multiple instances cannot run at the same time.
- Creates the initial state.
- Puts up the menu bar item.
- Grabs the keyboard, which swallows every key and hands it to the model as an event. The grab also hands back an emitter, which is how keys get back out.
- Subscribes to the other sources: the frontmost app, the event socket on `127.0.0.1:3883`, and SIGTERM.
- For each event, calls `state.handle(event)`, which gives back a vector of effects.
- For each effect, performs it.
