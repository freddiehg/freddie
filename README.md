# freddie

freddie is a framework for turning events into typed mutations of an application's state. laserbeam is a library within it (the typed mutable path). mercury is its first concrete use, a keyboard-remapping application that demonstrates freddie.

## Other realms

freddie can change keys easily, but that's not all! freddie is multi-talented and versatile.

## Where code goes

mercury is one consumer of freddie, not freddie itself. figaro is another, and there will be more. So the test for whether something belongs in mercury is whether figaro would write it differently: if figaro's copy would be identical, it does not belong in mercury, it belongs in a `freddie_*` crate that both depend on.

What mercury keeps is what is only true of mercury: its `App` enum, its state tree, its bindings, its effects, and the table mapping bundle ids onto its apps. What it does not keep is anything about how macOS works. Grabbing the keyboard, foregrounding an app, watching the frontmost app, and giving the main thread to a run loop are all identical in figaro, and each lives in its own crate.

This is easy to get wrong, because the first consumer is the only consumer and everything looks app-specific while it is the only thing there. The rule is about the second consumer, before it exists.

## Prior art

freddie's event loop follows two existing systems. isograph's language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. barnum goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. freddie's difference from barnum is that its handlers mutate state directly during dispatch, where barnum's only return a value the engine writes back. See `refactors/pending/event-loop.md` for detail.
