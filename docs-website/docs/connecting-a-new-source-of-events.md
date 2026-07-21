---
title: Connecting to a New Source of Events
sidebar_position: 4
---

# Connecting to a New Source of Events

A freddie program has one event type, whose variants are its sources:

```rust
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Tab(TabEvent),
    Quit(Quit),
    Timer(TimerFired),
}
```

Adding a source is three things: the event, the trigger that reads it, and the subscription that produces it. Nothing that binds a key has to hear about any of them.

## The event

An event is a plain struct carrying what the source knows and nothing else:

```rust
pub struct ForegroundEvent {
    pub app: App,
}
```

Then a variant on the enum. That variant is what `TryFrom` narrows on during dispatch, so the enum is the only place every source is listed together.

## The trigger

A trigger answers one question: does this event run this handler? It names the variant it reads and says what matching means.

```rust
pub trait EventTrigger {
    type Event;
    fn is_matching(&self, event: &Self::Event) -> bool;
}
```

The simplest one matches every event of its kind:

```rust
pub struct Foregrounded;

impl EventTrigger for Foregrounded {
    type Event = ForegroundEvent;
    fn is_matching(&self, _ev: &ForegroundEvent) -> bool {
        true
    }
}
```

Dispatch narrows the event to `Self::Event` with a `TryFrom` before it asks `is_matching`. So a key binding never sees a foreground event: the narrowing fails, the binding is skipped, and the trigger is never consulted. That is why adding a variant does not disturb the bindings already written.

Keys are the interesting case, because several triggers read the same event and differ in how much of it they look at. `Key::KeyR` matches that key on either press with any modifiers held. `Key::KeyR.down()` matches the direction too. `Key::KeyL.down().with(ModifierFlags::COMMAND)` matches the modifiers exactly, which is why Chrome binds `l`, `shift-l` and `cmd-l` as three separate chords rather than one key.

Where the trigger and the event are the same thing, `self_trigger!` writes the impl:

```rust
pub struct Quit;

bind::self_trigger!(Quit);
```

`Quit` carries nothing and means one thing, so there is no matching left to do.

## The subscription

The daemon owns the streams. Each one turns its items into the enum and sends them down a single channel, and the loop takes one event at a time off the other end.

```rust
freddie_app_nav::watch(move |bundle_id| {
    let app = App::from_bundle_id(&bundle_id);
    let _ = event_tx.send(MercuryEvent::Foreground(ForegroundEvent { app }));
});
```

A source that has to run to completion gets a task rather than a callback. SIGTERM is the one that does, because a `select!` arm that completed would drop the other futures and skip the shutdown it exists to run:

```rust
tokio::spawn(async move {
    if term.recv().await.is_some() {
        let _ = event_tx.send(quit_event());
    }
});
```

Sources may also be seeded rather than waited for. The model starts knowing which app is frontmost, so the in-app layer resolves correctly before the first foreground event arrives:

```rust
mercury.foreground.set_front_app(
    freddie_app_nav::frontmost()
        .map_or(App::Other, |id| App::from_bundle_id(&id)),
);
```

## Handling it

A binding for the new event goes wherever it is true, the same as a key. An event that keeps a piece of root state current binds at the root, and its handler returns no effect:

```rust
#[bind(
    Foregrounded => record_front_app,
)]
pub struct Mercury { /* ... */ }
```

That is the usual shape for a source: the event updates state, and the bindings that care about that state were written somewhere else entirely.
