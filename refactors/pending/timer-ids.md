# one timer event, matched by the guard that set it

Two problems, one shape.

Every timer mints its own type. `LayerTimeout` is a struct, a `MercuryTrigger` variant, a `MercuryEvent` variant, and a `self_trigger!`, all to say "the layer's idle timer went off"; `JkTimeout` is the same again, and the overlay's dwell would be a third. Nothing about any of it is per-timer except which timer fired.

And every timer races. Dropping a guard cancels the sleep, but a sleep that finished a moment earlier has already put its event on the channel, and that cannot be un-sent:

- `LayerTimeout` firing late sends you home from a layer you just entered.
- `JkTimeout` firing late interrupts a run that opened after the one it belonged to.
- The overlay's dwell firing late would hide the showing that superseded it.

Both fall out of one change: a firing says which timer it came from, and a binding matches only its own timer.

```rust
// one event, for every timer
pub struct TimerFired(pub TimerId);

// the binding names the guard whose firing it wants
#[bind(|nav_layer_path| nav_layer_path.get().home_timeout.trigger() => to_home)]
```

The identity does the work the per-timer types were doing, so the types go. The match does the work a stale-check in each handler would have done, so a stale firing matches no binding at all: dispatch returns `None`, the handler never runs, and no handler contains an `if` about it.

Built on two landed changes. `refactors/past/trigger-closures.md` lets a binding be written as a closure, which the derive calls with what dispatch is holding for that node; `refactors/past/path-get.md` makes that a shared reference, so a closure reads its node through `get`, reads the level above through `parent`, and cannot write either.

## why the ids are affordable

Ids are usually not worth it because of the bookkeeping: a counter on the state, a mint-and-bump wherever one is set, an id field on each event, and the discipline to keep them in step.

None of that is here. `timer_effect_and_guard` already builds the guard and the effect as a pair, so it mints the id and stamps both halves. A call site pays a closure:

```rust
let (guard, effect) = timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
self.overlay = Some(guard);
```

and a binding pays an expression naming its own guard. There is no counter to keep, and a guard that was replaced was dropped, so "is this event still mine" is answered by state that already exists.

## the trigger set becomes state-dependent, and that is fine

A binding whose node holds no guard produces a trigger that matches no firing, and two such triggers compare equal. Nothing goes wrong at dispatch, since neither matches anything. What it means is that the set THE CHECK collects depends on the state it walks: two nodes with no timer set look like one trigger, and `accumulate` would call that a duplicate.

Timer clobbering is deliberate (timer again replaces the guard, cancelling what it replaced), no-clobber is not a property the tree has yet, and `refactors/pending/no-clobber.md` is where it is decided. Nothing calls `accumulate` in mercury today.

## change 1: a timer guard, which is a drop guard plus which timer it is

`DropGuard` is a general RAII primitive: dropping it wakes a paired receiver, and it knows nothing about timers. It stays that way. What a node holds is a `TimerGuard`, which is that guard plus an id saying which timer it is, and the timer-shaped methods live there.

`crates/freddie/src/timer.rs`:

```rust
/// Identifies one timer.
///
/// Minted when the timer is set and stamped on both halves, so a fired event can be matched
/// against the timer still held. Process-wide and monotonic: two timers never share one, whoever
/// set them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId(u64);

impl TimerId {
    /// The next id.
    ///
    /// Atomic because a mutable static has to be `Sync`, not because anything sets timers off one
    /// thread; `Relaxed` because the only requirement is that no two calls return the same value.
    fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// The guard for one timer: what cancels it, and which timer it is.
///
/// Dropping it cancels the timer, because it drops the [`DropGuard`] inside it. Keeping it is what
/// lets a binding match the event this timer will fire and no other.
#[must_use = "dropping the guard cancels the timer immediately"]
#[derive(Debug)]
pub struct TimerGuard {
    id: TimerId,
    // Held only to be dropped: dropping it wakes the receiver the effect carries. Never read.
    #[expect(dead_code)]
    guard: DropGuard,
}

impl TimerGuard {
    /// The trigger matching this timer's own firing, and no other.
    #[must_use]
    pub const fn trigger(&self) -> TimerTrigger {
        TimerTrigger(Some(self.id))
    }
}
```

`drop_guard.rs` is untouched: no id, no timer, nothing to say about firings.

## change 2: the timer stamps the event

`crates/freddie/src/timer.rs`, before:

```rust
pub fn set_timer<E>(delay: Duration, event: E) -> (DropGuard, TimerEffect<E>) {
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
/// Build a linked timer and event that fires after `delay`. Dropping the timer cancels it.
///
/// `event` is handed the id identifying this timer, so the event it builds carries it and the
/// binding that wants it can match on the timer still held.
pub fn set_timer<E>(
    delay: Duration,
    event: impl FnOnce(TimerId) -> E,
) -> (Timer, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    let id = TimerId::mint();
    (
        TimerGuard { id, guard },
        TimerEffect {
            delay,
            event: event(id),
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

Its name is unchanged: it hands back a guard, as it did before, and only the event argument becomes a closure.

`crates/freddie/src/lib.rs` adds `TimerGuard` and `TimerId` to what it already re-exports.

## change 3: one event and one trigger, replacing three types

They live in `freddie`, beside the guard and the id they are about, the way `KeySequence` does and the way `freddie_keys` owns `Key` and `KeyEvent`. That is what lets a guard hand back its own trigger, so a binding never names a type at all. `freddie` takes a direct dependency on `bind` for `EventTrigger`; it already has one transitively through `freddie_keys`, and there is no cycle, since `bind` depends on `laserbeam` alone.

`crates/freddie/Cargo.toml` gains `bind = { path = "../bind", default-features = false }`, and the pair joins `crates/freddie/src/timer.rs` beside `Timer`:

```rust
/// A timer fired, carrying which timer it was.
///
/// One event for every timer a consumer owns. What tells them apart at dispatch is which node is
/// still holding that timer, not which type the event is.
#[derive(Debug)]
pub struct TimerFired(pub TimerId);

/// Matches only the firing of the timer it was built from.
///
/// Its value comes from the guard the bound node holds, so a binding written with it fires for its
/// own timer and nothing else. A node holding no guard produces `None`, which matches no firing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerTrigger(Option<TimerId>);

impl EventTrigger for TimerTrigger {
    type Event = TimerFired;
    fn is_matching(&self, ev: &TimerFired) -> bool {
        self.0 == Some(ev.0)
    }
}

impl TimerTrigger {
    /// The trigger matching the firing of the guard held here, or matching nothing when there is
    /// none: a state with no timer set has nothing to wait for.
    #[must_use]
    pub fn of(guard: Option<&TimerGuard>) -> Self {
        guard.map_or(Self(None), TimerGuard::trigger)
    }
}
```

`crates/freddie/src/lib.rs` re-exports the pair alongside the rest of the timer vocabulary.

mercury wraps them: `model.rs`, `MercuryTrigger` before:

```rust
    LayerTimeout(LayerTimeout),
    JkTimeout(JkTimeout),
```

after:

```rust
    TimerTrigger(TimerTrigger),
```

and `MercuryEvent` the same way, to `Timer(TimerFired)`, with `TimerFired`'s `testing` equality living in `freddie` (change 6). `sources.rs` loses both types rather than gaining any.

## change 4: each layer binds its own firing

The `Layer` node binds the return-home timeout for every variant today, so it would need to ask which variant is active and what guard it holds. It does not have to: the layers that set a timer are the ones that should bind its firing.

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
    // Only this layer's own timer: a firing from a nav already left matches nothing.
    |nav_layer_path| nav_layer_path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyC.down() => open_chrome,
    ..
)]
```

`resize.rs` and `app.rs` gain the same line, reading their own timer. The field becomes `pub(crate)` rather than growing an accessor: a binding needs to read it, and a `&TimerGuard` reached through a shared path can do nothing but name its own firing.

```rust
pub struct NavLayer {
    // Read for the trigger that matches its firing, and held for its `Drop`: dropping the guard
    // cancels nav's return-home timer.
    pub(crate) home_timeout: TimerGuard,
}
```

The field is renamed from `timeout` on all three layers, since a binding reads it now and "the timeout" says nothing about which one; its `#[expect(dead_code)]` goes with the rename, because it is read. No layer mentions `TimerId`.

A closure's parameter is named for the path it is: `nav_layer_path`, not `path` or `nav`. It is neither the layer nor a generic path, and `nav.get()` reads as though it were the layer itself.

`Home` and `Typing` set no timer and bind nothing, so there is no `None` case anywhere: the absence is the absent binding.

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

The sequence exposes the guard its live run holds, in `crates/freddie/src/sequence.rs`:

```rust
    /// The guard for a live run's window timer, or `None` when no run is live or this sequence has no
    /// window. What a binding matches against, so a firing from a run that has since ended matches
    /// nothing.
    #[must_use]
    pub fn window_timer(&self) -> Option<&TimerGuard> {
        self.window.as_ref()?.timer.as_ref()
    }
```

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
    |mercury_path| TimerTrigger::of(mercury_path.typing_state.jk.window_timer()) => jk_timeout,
    AnyKey => maybe_pass_through,
```

The root's path is `&mut Mercury`, so its closure reads fields straight through the deref; a layer's is a `PathMut`, so its closure reads the layer through `get`.

Both helpers that set one take the closure form: `|id| MercuryEvent::Timer(TimerFired(id))`.

The handlers lose their event types: `to_home` is already generic over the event, and `jk_timeout` in `handlers/root.rs` takes `&TimerFired` (its `use crate::JkTimeout` becomes `use crate::TimerFired`). Neither needs a staleness check, because a stale firing no longer reaches them.

## change 6: the tests, and what a timer effect compares as

26 assertions in `crates/mercury/tests/transitions.rs` rebuild a timer effect (`return_home_timer()`, `jk_timer()`) and compare it to what a transition produced. A minted id breaks every one, and rebuilding cannot fix it: the id is unpredictable by construction.

So `TimerFired` compares equal to any other under `testing`, the way `AlwaysEqual` does. In `crates/freddie/src/timer.rs`, beside the type:

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

Two helpers replace `jk_window_fired()`, since a test can no longer name a firing without having watched the timer be set:

```rust
// A firing of `id`, which a test reads off the effect the timer produced: nothing else can know
// it, because the timer mints it.
const fn fired(id: freddie::TimerId) -> MercuryEvent {
    MercuryEvent::Timer(TimerFired(id))
}

// The id a timer effect was set with.
fn timer_id(effect: &MercuryEffect) -> freddie::TimerId {
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
