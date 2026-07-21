---
title: The Event Loop
sidebar_position: 2
---

# The Event Loop

The setup side of a freddie program is ordinary code, and it is the same few steps every time.

Subscribe each source, handing it a callback that does one thing: turn what arrived into a variant of the program's event enum and send it into one queue. Sources push, and nothing polls. A keyboard tap, an app watcher and a signal handler each run wherever the OS puts them, and the queue is where they meet.

Drain that queue one event at a time. An iteration takes one event and hands it to `state.handle(&event)`, which dispatches it, mutates the state through the path the winning handler was given, and returns the effects that handler asked for. One thread owns the state and is the only one that calls `handle`, so nothing guards it.

Then perform the effects, in the order dispatch produced them, which is why a modifier reaches the OS before the key carrying its flag. An effect that touches the world is performed here and never inside a handler. An effect with a result does not return it: the result arrives later as its own event, from whichever source observes it, which is why foregrounding an app and seeing it come up are two separate things to the model.

The loop is small enough to read at once:

```rust
while let Some(event) = event_rx.recv().await {
    let effects = state.handle(&event).unwrap_or_default();
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}
```

`handle` returns `None` when nothing on the active path bound the event, and that is the ordinary case rather than an error: a program subscribes to every event it may ever want, not to the ones its current state happens to bind. The effect loop is the second consumer, reading the effect channel and performing one effect per iteration until one of them says to stop. The two run together under a `select!`, so `Kill` ending the effect loop ends the program.

## Prior art

freddie's event loop follows two existing systems. [`isograph`](https://github.com/isographlabs/isograph)'s language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. [`barnum`](https://github.com/barnum-circus/barnum) goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. freddie's difference from `barnum` is that its handlers mutate state directly during dispatch, where `barnum`'s only return a value the engine writes back.

## `mercury`'s setup

`mercury` uses `clap` to parse commands. The main subcommand is `mercury start`, which calls a hidden internal command, `mercury daemon`, which does the following:

- Takes the single-instance lock, so multiple instances cannot run at the same time.
- Creates the initial state.
- Puts up the menu bar item.
- Grabs the keyboard, which swallows every key and hands it to the model as an event. The grab also hands back an emitter, which is how keys get back out.
- Subscribes to the other sources: the frontmost app, external events, and SIGTERM.
- For each event, calls `state.handle(event)`, which gives back a vector of effects.
- For each effect, performs it.
