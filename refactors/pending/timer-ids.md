# one timer event, matched by the guard that armed it

Two problems, one shape. Verified against the tree: the mechanism it rests on works today (see below).

Every timer mints its own type. `LayerTimeout` is a struct, a `MercuryTrigger` variant, a `MercuryEvent` variant, and a `self_trigger!`, all to say "the layer's idle timer went off"; `JkTimeout` is the same again, and the overlay's dwell would be a third. Nothing about any of it is per-timer except which timer fired.

And every timer races. Dropping a guard cancels the sleep, but a sleep that finished a moment earlier has already put its event on the channel, and that cannot be un-sent:

- `LayerTimeout` firing late sends you home from a layer you just entered.
- `JkTimeout` firing late interrupts a run that opened after the one it belonged to.
- The overlay's dwell firing late hides the showing that superseded it.

Both fall out of one change: a firing carries the identity of the arming it came from, and a binding matches only the firing its own guard is waiting on.

```rust
// one event, for every timer
pub struct TimerFired(pub TimerId);

// the binding names the guard whose firing it wants
#[bind(ArmedTimer(path.overlay.as_ref().map(DropGuard::id)) => hide_overlay)]
```

The identity does the work the per-timer types were doing, so the types go. The match does the work a stale-check in each handler would have done, so a stale firing matches no binding at all: dispatch returns `None`, the handler never runs, and no handler contains an `if` about it.

## why this is affordable

Ids are usually not worth it because of the bookkeeping: a counter on the state, a mint-and-bump at each arm site, an id field on each event, and the discipline to keep them in step.

None of that is here. `timer_effect_and_guard` already builds the guard and the effect as a pair, so it mints the id and stamps both halves. A call site pays a closure:

```rust
let (guard, effect) = timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
self.overlay = Some(guard);
```

and a binding pays an expression naming its own guard. There is no counter to keep, and a guard that was replaced was dropped, so "is this event still mine" is answered by the state that already exists.

## what makes it possible

A trigger is not a constant. `bind_macro` parses it as a `syn::Expr` and emits `let trigger = #trigger;` INSIDE the dispatch body, where `path` is in scope and has not yet been moved into the handler. So a trigger can read the node it is bound on.

Verified, not assumed. A trigger bound as `Armed(path.armed_id) => on_fired`, against a root field `armed_id: Option<u64>`, compiles and dispatches: an unarmed node matched no firing, a stale id matched no firing, and its own id ran the handler.

But `path` is not a name `bind` offers. It is the identifier the generated `dispatch` happens to bind its parameter as, so a trigger reading it captures a macro-internal name unhygienically, and a derived level's generated body binds `node` instead. Neither is documented, and renaming either inside `bind_macro` would silently break every binding that read it.

So this takes a prefactor: a trigger may be written as a CLOSURE, and the macro calls it with the node's own struct rather than evaluating it.

```rust
#[bind(
    |root| ArmedTimer(root.overlay_id()) => hide_overlay,
    Quit => quit,
)]
```

The binding names its own parameter, so nothing is captured invisibly, and a constant trigger is written exactly as it is today. It needs no new grammar either: `binds()` already parses each trigger as a `syn::Expr`, and a closure is one, so this is a `match` on `Expr::Closure` in the emit.

What it is called with is a read-only view over the node it is bound on:

```rust
/// What a trigger closure is handed: read access to the node it is bound on, and to its parent.
///
/// Not the path itself, because a trigger has no business mutating the tree, and not `&PathMut`,
/// because reading the node needs the unique borrow its projection re-derives it through.
pub struct PathView<'a, N, P>(&'a mut PathMut<N, P>);

impl<N, P> PathView<'_, N, P> {
    /// The node this trigger is bound on. Takes `&mut self` because the projection does, not
    /// because it writes anything.
    pub fn node(&mut self) -> &N {
        self.0.get_mut()
    }

    /// The level above, for a trigger whose answer lives further up.
    pub const fn parent(&self) -> &P {
        self.0.parent()
    }
}
```

```rust
#[bind(|nav| ArmedTimer(nav.node().return_home_id()) => to_home)]
```

`node()` and `parent()` cannot be held at once, because `PathMut`'s projection re-derives the node FROM the parent, so borrowing one borrows the path. Sequentially is fine, and a trigger computing a value never needs both live.

The root needs none of it: its path is `&mut Mercury` already, so the emit is `(#closure)(&*path)` and the closure reads fields directly. A deeper place node gets `(#closure)(PathView(&mut path))`, and a derived level gets `(#closure)(&node.data)`, its own struct, since it has no path to view.

The reborrow ends before `path` or `node` moves into the handler, so the closure form composes with what dispatch already does.

## the trigger set becomes state-dependent, and that is fine

`ArmedTimer(None)` is what a binding produces when its node holds no guard, and two of them compare equal. Nothing goes wrong at dispatch, because `is_matching` is `self.0 == Some(ev.0)` and `None` matches no firing. What it means is that the set of triggers THE CHECK collects depends on the state it walks: two unarmed timers look like the same trigger, and `accumulate` would call that a duplicate.

That is not a reason to avoid this. Timer clobbering is deliberate here (arming again replaces the guard, cancelling what it replaced), and no-clobber is not a property the tree has yet: `refactors/pending/no-clobber.md` is the doc for making overlap a stated thing rather than an accident, and this is one more input to it. Nothing calls `accumulate` in mercury today, so nothing breaks in the meantime.

What that doc will have to say about this: a trigger whose value is read from state answers "no duplicates in this state", not "no duplicates ever", so either the check learns to skip such triggers or it walks the states it cares about.

## change 1: the id, minted with the pair

`crates/freddie/src/drop_guard.rs`:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// Identifies one arming of one timer.
///
/// Minted by [`timer_effect_and_guard`](crate::timer_effect_and_guard) and stamped on both halves,
/// so a fired event can be matched against the guard still held. Process-wide and monotonic: two
/// armings never share one, whoever armed them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId(u64);

impl TimerId {
    /// The next id. Relaxed: the only requirement is that no two calls return the same value.
    fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
```

`DropGuard` before:

```rust
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard(
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[expect(dead_code)] oneshot::Sender<()>,
);
```

after:

```rust
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard {
    id: TimerId,
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[expect(dead_code)]
    cancel: oneshot::Sender<()>,
}

impl DropGuard {
    /// The arming this guard is waiting on, for a binding that matches only its own firing.
    #[must_use]
    pub const fn id(&self) -> TimerId {
        self.id
    }
}
```

```rust
/// Build a linked guard/receiver pair, and the id identifying this arming.
pub fn drop_guard() -> (DropGuard, oneshot::Receiver<()>, TimerId) {
    let (cancel, receiver) = oneshot::channel();
    let id = TimerId::mint();
    (DropGuard { id, cancel }, receiver, id)
}
```

## change 2: the timer stamps the event

`crates/freddie/src/timer.rs`, before:

```rust
pub fn timer_effect_and_guard<E>(delay: Duration, event: E) -> (DropGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    (
        guard,
        TimerEffect {
            delay,
            event,
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

after:

```rust
/// Build a linked guard/event pair that fires after `delay`. The guard cancels the timer on drop.
///
/// `event` is handed the id identifying this arming, so the event it builds carries it and the
/// binding that wants it can match on the guard still held.
pub fn timer_effect_and_guard<E>(
    delay: Duration,
    event: impl FnOnce(TimerId) -> E,
) -> (DropGuard, TimerEffect<E>) {
    let (guard, receiver, id) = drop_guard();
    (
        guard,
        TimerEffect {
            delay,
            event: event(id),
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

`crates/freddie/src/lib.rs` adds `TimerId` to the `drop_guard` re-export.

## change 3: one event and one trigger in mercury

`crates/mercury/src/sources.rs`, replacing `LayerTimeout` and `JkTimeout`:

```rust
/// A timer fired, carrying the arming it came from.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerFired(pub TimerId);

/// Matches only the firing of the arming it was built from.
///
/// Its value comes from the guard the bound node holds, so a binding written with it fires for
/// its own timer and for nothing else. A node holding no guard produces `None`, which matches no
/// firing at all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ArmedTimer(pub Option<TimerId>);

impl EventTrigger for ArmedTimer {
    type Event = TimerFired;
    fn is_matching(&self, ev: &TimerFired) -> bool {
        self.0 == Some(ev.0)
    }
}
```

`model.rs` drops `LayerTimeout` and `JkTimeout` from both enums and gains `ArmedTimer(ArmedTimer)` on the trigger and `Timer(TimerFired)` on the event.

## change 4: the bindings name their guards

`crates/mercury/src/state/mod.rs`. `Layer`, before:

```rust
#[bind(LayerTimeout => to_home)]
```

after, where each variant's guard supplies the id:

```rust
#[bind(ArmedTimer(path.get().return_home_id()) => to_home)]
```

which needs `Layer::return_home_id(&self) -> Option<TimerId>`, returning the held guard's id for the three layers that arm one and `None` for typing and home.

The root, before:

```rust
    JkTimeout => jk_timeout,
```

after:

```rust
    ArmedTimer(path.typing_state.jk_id()) => jk_timeout,
```

which needs `KeySequence` to expose the id of the guard a live run holds.

Both arm sites take the closure form: `|id| MercuryEvent::Timer(TimerFired(id))`.

## change 5: the tests, and what a timer effect compares as

26 assertions in `crates/mercury/tests/transitions.rs` rebuild a timer effect (`return_home_timer()`, `jk_timer()`) and compare it to what a transition produced. A minted id would break every one of them, and rebuilding cannot fix it: the id is unpredictable by construction.

So `TimerFired` compares equal to any other under `testing`, the way `AlwaysEqual` does:

```rust
/// Two firings compare equal under `testing` whatever their ids.
///
/// The id exists to tell one ARMING from another at dispatch; a test that rebuilds an expected
/// effect cannot know it, and asserting it would only assert that the counter ran. With one event
/// type for every timer, the delay is what distinguishes an effect anyway.
#[cfg(feature = "testing")]
impl PartialEq for TimerFired {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(feature = "testing")]
impl Eq for TimerFired {}
```

The 26 assertions then stand unchanged, and the id is asserted where it means something: read off the effect a transition produced, and driven back in as a firing.

Cases worth having beyond the existing ones:

- a stale firing matches nothing: arm, supersede, then fire the first arming's id and assert `handle` returns `None`, since no binding matched.
- a live firing still fires: fire the id the current guard holds and assert the timeout's effects.
