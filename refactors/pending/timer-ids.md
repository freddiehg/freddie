# one timer event, matched by the guard that armed it

Two problems, one shape.

Every timer mints its own type. `LayerTimeout` is a struct, a `MercuryTrigger` variant, a `MercuryEvent` variant, and a `self_trigger!`, all to say "the layer's idle timer went off"; `JkTimeout` is the same again, and the overlay's dwell would be a third. Nothing about any of it is per-timer except which timer fired.

And every timer races. Dropping a guard cancels the sleep, but a sleep that finished a moment earlier has already put its event on the channel, and that cannot be un-sent:

- `LayerTimeout` firing late sends you home from a layer you just entered.
- `JkTimeout` firing late interrupts a run that opened after the one it belonged to.
- The overlay's dwell firing late would hide the showing that superseded it.

Both fall out of one change: a firing carries the identity of the arming it came from, and a binding matches only the firing its own guard is waiting on.

```rust
// one event, for every timer
pub struct TimerFired(pub TimerId);

// the binding names the guard whose firing it wants
#[bind(|root| ArmedTimer(root.overlay_id()) => hide_overlay)]
```

The identity does the work the per-timer types were doing, so the types go. The match does the work a stale-check in each handler would have done, so a stale firing matches no binding at all: dispatch returns `None`, the handler never runs, and no handler contains an `if` about it.

Built on `refactors/past/trigger-closures.md`, which landed: a binding may be written as a closure and the derive calls it with the node's path, so `|root| ArmedTimer(root.overlay_id())` is a supported form rather than a capture of a name the macro happens to use.

## why the ids are affordable

Ids are usually not worth it because of the bookkeeping: a counter on the state, a mint-and-bump at each arm site, an id field on each event, and the discipline to keep them in step.

None of that is here. `timer_effect_and_guard` already builds the guard and the effect as a pair, so it mints the id and stamps both halves. A call site pays a closure:

```rust
let (guard, effect) = timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
self.overlay = Some(guard);
```

and a binding pays an expression naming its own guard. There is no counter to keep, and a guard that was replaced was dropped, so "is this event still mine" is answered by state that already exists.

## the trigger set becomes state-dependent, and that is fine

`ArmedTimer(None)` is what a binding produces when its node holds no guard, and two of them compare equal. Nothing goes wrong at dispatch, because `is_matching` is `self.0 == Some(ev.0)` and `None` matches no firing. What it means is that the set THE CHECK collects depends on the state it walks: two unarmed timers look like one trigger, and `accumulate` would call that a duplicate.

Timer clobbering is deliberate (arming again replaces the guard, cancelling what it replaced), no-clobber is not a property the tree has yet, and `refactors/pending/no-clobber.md` is where it is decided. Nothing calls `accumulate` in mercury today.

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
    /// The next id.
    ///
    /// Atomic because a mutable static has to be `Sync`, not because anything arms timers off one
    /// thread; `Relaxed` because the only requirement is that no two calls return the same value.
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

## change 3: one event and one trigger, replacing three types

`crates/mercury/src/sources.rs`, which gains `use freddie::TimerId`. `LayerTimeout` and `JkTimeout` are deleted, and this replaces both:

```rust
/// A timer fired, carrying the arming it came from.
///
/// One event for every timer mercury owns. What distinguishes them at dispatch is which guard is
/// still holding the arming, not which type the event is.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerFired(pub TimerId);

/// Matches only the firing of the arming it was built from.
///
/// Its value comes from the guard the bound node holds, so a binding written with it fires for its
/// own timer and nothing else. A node holding no guard produces `None`, which matches no firing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ArmedTimer(pub Option<TimerId>);

impl EventTrigger for ArmedTimer {
    type Event = TimerFired;
    fn is_matching(&self, ev: &TimerFired) -> bool {
        self.0 == Some(ev.0)
    }
}
```

`model.rs`, `MercuryTrigger` before:

```rust
    LayerTimeout(LayerTimeout),
    JkTimeout(JkTimeout),
```

after:

```rust
    ArmedTimer(ArmedTimer),
```

and `MercuryEvent` the same way, to `Timer(TimerFired)`. `lib.rs` re-exports `ArmedTimer` and `TimerFired` in place of the two.

## change 4: each layer binds its own firing

The `Layer` node binds the return-home timeout for every variant today, so it would need to ask which variant is active and what guard it holds. It does not have to: the layers that arm a timer are the ones that should bind its firing.

`crates/mercury/src/state/nav.rs`, before:

```rust
#[bind(
    Key::Escape.down() => to_home,
    Key::KeyC.down() => open_chrome,
    ..
)]
```

after:

```rust
#[bind(
    // Only this layer's own arming: a firing from a nav already left matches nothing.
    |nav| ArmedTimer(Some(nav.get_mut().timeout_id())) => to_home,
    Key::Escape.down() => to_home,
    Key::KeyC.down() => open_chrome,
    ..
)]
```

`resize.rs` and `app.rs` gain the same line, reading their own `timeout_id()`. Each of the three exposes it:

```rust
    /// The return-home arming this layer is waiting on, for the binding that matches only its own
    /// firing.
    #[must_use]
    pub(crate) const fn timeout_id(&self) -> TimerId {
        self.timeout.id()
    }
```

Each takes `TimerId` into its `use freddie::{..}`, and each `timeout` field drops its `#[expect(dead_code)]`: it is read now. The comment above it changes with it, from "held for its `Drop`" to being read for its id as well.

`Home` and `Typing` arm nothing and bind nothing, so there is no `None` case anywhere: the absence is the absent binding.

`crates/mercury/src/state/mod.rs`, the `Layer` node, before:

```rust
#[bind(LayerTimeout => to_home)]
pub enum Layer {
```

after, binding nothing at all, since `escape` already moved down to the command layers:

```rust
pub enum Layer {
```

## change 5: the root binds the jk window

`crates/mercury/src/state/mod.rs`, `Mercury`'s `#[bind(..)]`, before:

```rust
    Quit => quit,
    JkTimeout => jk_timeout,
    AnyKey => maybe_pass_through,
```

after:

```rust
    Quit => quit,
    // Only this run's window: a firing from a run that has since ended matches nothing, so the
    // handler never sees it.
    |root| ArmedTimer(root.typing_state.jk.window_id()) => jk_timeout,
    AnyKey => maybe_pass_through,
```

The root's path is `&mut Mercury`, so its closure reads fields directly; a layer's is a `PathMut`, so its closure reads through `get_mut`.

Both arm helpers take the closure form: `|id| MercuryEvent::Timer(TimerFired(id))`.

The handlers lose their event types: `to_home` is already generic over the event, and `jk_timeout` in `handlers/root.rs` takes `&TimerFired` (its `use crate::JkTimeout` becomes `use crate::TimerFired`). Neither needs a staleness check, because a stale firing no longer reaches them.

## change 6: the tests, and what a timer effect compares as

26 assertions in `crates/mercury/tests/transitions.rs` rebuild a timer effect (`return_home_timer()`, `jk_timer()`) and compare it to what a transition produced. A minted id breaks every one, and rebuilding cannot fix it: the id is unpredictable by construction.

So `TimerFired` compares equal to any other under `testing`, the way `AlwaysEqual` does:

```rust
/// Two firings compare equal under `testing` whatever their ids.
///
/// The id exists to tell one ARMING from another at dispatch. A test that rebuilds an expected
/// effect cannot know it, and asserting it would only assert that the counter ran; with one event
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

Two helpers replace `jk_window_fired()`, since a test can no longer name a firing without having watched one be armed:

```rust
// A firing of `id`, which a test reads off the effect the arming produced: nothing else can know
// it, because the timer mints it.
const fn fired(id: freddie::TimerId) -> MercuryEvent {
    MercuryEvent::Timer(TimerFired(id))
}

// The id a timer effect was armed with.
fn armed_id(effect: &MercuryEffect) -> freddie::TimerId {
    match effect {
        MercuryEffect::Timer(timer) => match timer.event {
            MercuryEvent::Timer(TimerFired(id)) => id,
            ref other => panic!("not a timer firing: {other:?}"),
        },
        other => panic!("not a timer effect: {other:?}"),
    }
}
```

Every existing case that drives a firing reads the id first: `nav_times_out_home` and `layer_timeout_returns_home_from_any_layer` take it off the effect entering the layer produced, and the `jk` window cases off the effect the opening `j` produced.

New cases:

- a stale firing matches nothing. Arm a return-home by entering nav, read its id off the effect, leave and re-enter so a fresh timer supersedes it, then fire the first id and assert `handle` returns `None`, since no binding matched.
- a live firing still fires: fire the id the current guard holds, and assert the layer went home.
- the same pair for the `jk` window: open a run, read its id, break the run, open another, and assert the first id does nothing while the second replays.
