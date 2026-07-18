# state that hides itself when it goes away

A stub. The problem is real and the shape of the answer is not settled.

## the problem

Some state has a visible counterpart that has to be taken down when the state goes away. The overlay is the first: `overlay: Option<TimerGuard>` says one is on screen, and every path that clears it has to push `HideOverlay` by hand — the dwell firing, a layer change, and (see `refactors/pending/overlay.md`) whatever answers the front-app staleness question.

Forgetting one leaves a window on screen that the model believes is gone.

RAII is what this asks for, and Rust cannot quite give it. `Drop::drop` returns nothing, so a guard cannot hand an effect back to the dispatch that dropped it. A guard could hold a channel sender and push out of band — that is what `DropGuard` does for a timer — but an effect sent that way is unordered against the batch the handler is building, and for a shared resource like the one overlay panel, order decides whether you end up showing or hiding it. A timer's cancellation is order-insensitive; a panel's is not.

## what we do today

`layer` is the precedent: private, written only through `set_layer`, which returns the modifier flush and is `#[must_use]`. You cannot change the layer without producing the effects that change implies, and you cannot discard them silently.

The overlay can follow it: private field, a small set of methods that are the only writers, each returning its effects, each `#[must_use]`. That makes the mistake a compile error rather than a habit, which is most of the value.

## what is open

- Whether "private field plus `#[must_use]` methods" is the whole answer, repeated per case, or whether there is one abstraction worth naming — something like a `Tracked<T>` whose setter returns the effects its change implies.
- Whether the effect loop could instead reconcile: the model states what should be on screen, and the loop diffs it against what is, so nothing has to remember to hide anything. That trades the ordering problem for a diff, and it is a different architecture from the one `effects-and-events.md` describes.
- Whether anything but the overlay wants this. One case is not a pattern; the menu bar title is close (it is a `ShowLayer` effect pushed by `set_layer`), and a second would say which way to go.
