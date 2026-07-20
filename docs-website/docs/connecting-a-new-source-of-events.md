---
title: Connecting to a New Source of Events
sidebar_position: 4
---

# Connecting to a New Source of Events

A `freddie` program has one event type, whose variants are its sources:

```rust
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Tab(TabEvent),
    Quit(Quit),
    Timer(TimerFired),
}
```

Adding a source is adding a variant and the triggers that read it. Nothing that binds a key has to hear about it.

## Triggers

A trigger answers one question: does this event run this handler? It names the variant it reads and says what matching means:

```rust
pub trait EventTrigger {
    type Event;
    fn is_matching(&self, event: &Self::Event) -> bool;
}
```

Dispatch narrows the event to `&Self::Event` first, with a `TryFrom`, and asks `is_matching` only if that succeeded. So a key binding never sees a tab event: the narrowing fails, the binding is skipped, and the trigger never runs.

## Subscribing a new source

TODO: show the setup side — where in `mercury daemon` a stream gets subscribed, how its items are turned into the event enum, and how it joins the other sources in the select.

## Writing the trigger

TODO: implement `EventTrigger` for the new event, including the `TryFrom` that narrows to it.

## Trigger closures

A trigger does not have to be a constant. It can be a closure over the state its node is bound on:

```rust
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
)]
```

Every timer fires the same `MercuryEvent::Timer`, so the event alone cannot say which one went off. The layer holds the guard for the timer it set, and that guard's `trigger()` matches its own firing and nothing else. The closure is handed a shared reference, so a trigger reads state and cannot write it.

## The event socket

TODO: the loopback WebSocket on `127.0.0.1:3883`, the frame format, and how the Chrome extension uses it.
