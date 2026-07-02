# freddie

freddie is a framework for turning events into typed mutations of an application's state. laserbeam is a library within it (the typed mutable path). mercury is its first concrete use, a keyboard-remapping application that demonstrates freddie.

## Other realms

freddie can change keys easily, but that's not all! freddie is multi-talented and versatile.

## Prior art

freddie's event loop follows two existing systems. isograph's language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. barnum goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. freddie's difference from barnum is that its handlers mutate state directly during dispatch, where barnum's only return a value the engine writes back. See `refactors/pending/event-loop.md` for detail.
