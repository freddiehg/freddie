# state that hides itself when it goes away

A stub. The problem is real and the shape of the answer is not settled.

## the problem

Some state has a visible counterpart that has to be taken down when the state goes away. The overlay is the first: `overlay: Option<TimerGuard>` says one is on screen, and every path that clears it has to push `HideOverlay` by hand — the dwell firing, a layer change, and (see `refactors/pending/overlay.md`) whatever answers the front-app staleness question.

Forgetting one leaves a window on screen that the model believes is gone.

RAII is what this asks for, and Rust cannot quite give it. `Drop::drop` returns nothing, so a guard cannot hand an effect back to the dispatch that dropped it. A guard could hold a channel sender and push out of band — that is what `DropGuard` does for a timer — but an effect sent that way is unordered against the batch the handler is building, and for a shared resource like the one overlay panel, order decides whether you end up showing or hiding it. A timer's cancellation is order-insensitive; a panel's is not.

## what we do today

`layer` is the precedent: private, written only through `set_layer`, which returns the modifier flush and is `#[must_use]`. You cannot change the layer without producing the effects that change implies, and you cannot discard them silently.

The overlay can follow it: private field, a small set of methods that are the only writers, each returning its effects, each `#[must_use]`. That makes the mistake a compile error rather than a habit, which is most of the value.

## the shape worth trying

Repeating "private field, `#[must_use]` setter" per case is discipline wearing a type's clothes. Naming it once is not:

```rust
/// State with a visible counterpart, which is taken down when the state goes.
///
/// The value is private and reachable only through the two methods, and both hand back the
/// effects their change implies, so there is no way to put something on screen without showing it
/// or to drop it without hiding it.
pub struct Shown<T: Visible>(Option<T>);

/// What a piece of state's counterpart looks like, and what taking it down means.
pub trait Visible {
    type Effect;
    /// The effect that puts this on screen.
    fn shown(&self) -> Self::Effect;
    /// The effect that takes down whatever is on screen.
    fn hidden() -> Self::Effect;
}

impl<T: Visible> Shown<T> {
    /// Show `value`, replacing whatever was up. No hide: the counterpart is shared, so the new
    /// showing overwrites it rather than the old one being taken down first.
    #[must_use = "the returned effect is what puts it on screen"]
    pub fn show(&mut self, value: T) -> T::Effect {
        let effect = value.shown();
        self.0 = Some(value);
        effect
    }

    /// Take it down, if anything is up.
    #[must_use = "the returned effect is what takes it off the screen"]
    pub fn hide(&mut self) -> Option<T::Effect> {
        self.0.take().map(|_| T::hidden())
    }
}
```

The overlay's `T` carries its dwell guard and its text; `shown()` is `ShowOverlay(text)` and `hidden()` is `HideOverlay`. `set_layer` calls `hide()` and appends what it returns, which is the same line it writes today, except that not writing it is now impossible rather than merely noticed in review.

What it still cannot do is fire on an ordinary drop — replacing the whole `Shown` wholesale, or dropping the struct that holds it, skips the effect. The same answer as `layer` applies: the field stays private to the root, and the root's own methods are the only writers.

## what is open

- Whether one case justifies the type. The overlay is the only one today; the menu bar title is the near miss, and it works without any of this because a title is always overwritten and never taken down.
- Whether the effect loop could instead reconcile: the model states what should be on screen, and the loop diffs it against what is, so nothing has to remember to hide anything. That trades the ordering problem for a diff, and it is a different architecture from the one `effects-and-events.md` describes.
- Whether anything but the overlay wants this. One case is not a pattern; the menu bar title is close (it is a `ShowLayer` effect pushed by `set_layer`), and a second would say which way to go.
