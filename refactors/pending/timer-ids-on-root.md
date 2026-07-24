# timer ids on the root

The source of timer ids is a process-global `static AtomicU64` in `crates/freddie/src/timer.rs`. It is ambient, and it makes every dispatch that arms a timer impure: the id a transition mints depends on how many timers were armed before, not on the state alone. Under `CLAUDE.md`'s ambient-state rule the counter belongs on the root model, as a plain field threaded by `&mut` to the arming sites.

Moving it there makes the id a function of state, which pays off in the model tests: `TimerFired`'s `testing`-only "always equal" impl exists only because a test could not predict the id a global counter handed out. With the counter on the root, the id in a produced effect is the root's counter before the arm, which a test that builds the expected state knows, so the id becomes assertable and the hack goes.

The same change retires the per-call closure: every arming site builds `Event::Timer(TimerFired(id))`, so a `From<TimerFired>` bound expresses it once.

## The id source newtype

`crates/freddie/src/timer.rs`, replacing `mint`. The `use std::sync::atomic::{AtomicU64, Ordering};` at the top goes.

before:

```rust
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
```

after:

```rust
/// The source of timer ids, one per model. Held on the root and advanced when a timer is armed, so
/// the id a dispatch hands out is a function of state rather than of a process-global counter.
///
/// Monotonic and never reset within a run: a firing from a cancelled timer must never carry the id
/// of one armed later, or a stale event would match a fresh guard. `next` bumps on every call, so a
/// minted id and the advance are one event; there is no `peek`. `TimerGuard` and `TimerEffect` are
/// built from the id in the same call, so holding a guard implies the bump.
///
/// `&mut self` on `next` is the mutual exclusion: only one `&mut TimerIds` exists at a time, so two
/// mints cannot read the same value before either advances. No `RefCell` or atomic is needed.
///
/// Not `Clone`: a copy would hand out ids whose advance never reached the root, the one way two
/// timers could share an id.
#[derive(Default, Debug)]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
pub struct TimerIds(u64);

impl TimerIds {
    fn next(&mut self) -> TimerId {
        let id = TimerId(self.0);
        self.0 += 1;
        id
    }
}
```

## `timer_effect_and_guard` takes the source

The closure `impl FnOnce(TimerId) -> E` existed so the caller could bake the minted id into an event. Every caller built `Event::Timer(TimerFired(id))`, so a `From<TimerFired>` bound expresses it once and the closure goes.

before:

```rust
pub fn timer_effect_and_guard<E>(
    delay: Duration,
    event: impl FnOnce(TimerId) -> E,
) -> (TimerGuard, TimerEffect<E>) {
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

after:

```rust
pub fn timer_effect_and_guard<E: From<TimerFired>>(
    timer_ids: &mut TimerIds,
    delay: Duration,
) -> (TimerGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    let id = timer_ids.next();
    (
        TimerGuard { id, guard },
        TimerEffect {
            delay,
            event: E::from(TimerFired(id)),
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

The return type is unchanged; the use site wraps it into `MercuryEffect::Timer(...)` as it already did, and `E` is inferred from that wrap, which requires `TimerEffect<MercuryEvent>`, so no call needs a turbofish. No custom trait is introduced: a one-implementor trait returning the consumer's effect directly would only fold a wrap that is not duplicated, against `CLAUDE.md`'s rule on custom traits.

`crates/mercury/src/model.rs`, added beside `MercuryEvent`:

```rust
impl From<TimerFired> for MercuryEvent {
    fn from(fired: TimerFired) -> Self {
        MercuryEvent::Timer(fired)
    }
}
```

`crates/freddie/src/lib.rs` re-exports `TimerIds` alongside the existing timer exports.

## `TimerFired` derives its comparison again

`crates/freddie/src/timer.rs`, before:

```rust
/// A timer fired, carrying which timer it was.
///
/// One event for every timer a consumer owns. What tells them apart at dispatch is which node is
/// still holding that guard, not which type the event is.
#[derive(Debug)]
pub struct TimerFired(pub TimerId);

/// Two firings compare equal under `testing` whatever their ids.
///
/// The id exists to tell one timer from another at dispatch. A test that rebuilds an expected
/// effect cannot know it, and asserting it would only assert that the counter ran; with one event
/// type for every timer, the delay is what distinguishes an effect anyway. A test that cares about
/// an id reads it off the effect a transition produced.
#[cfg(feature = "testing")]
impl PartialEq for TimerFired {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(feature = "testing")]
impl Eq for TimerFired {}
```

after:

```rust
/// A timer fired, carrying which timer it was.
///
/// One event for every timer a consumer owns. What tells them apart at dispatch is which node is
/// still holding that guard, not which type the event is.
///
/// The id is assertable under `testing`: it is the root's [`TimerIds`] before the arm, so a test
/// that builds the expected state knows exactly which id a transition mints, and comparing it
/// asserts the counter advanced by the right amount.
#[derive(Debug)]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
pub struct TimerFired(pub TimerId);
```

`TimerEffect` keeps its `#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]` and its `AlwaysEqual<oneshot::Receiver<()>>` on `cancel`: the receiver is incomparable whatever the id does, so that wrapper is unrelated to this change and stays. Its `event` field now compares by the real id, since `TimerFired` does. This does not give `MercuryEffect` or `MercuryEvent` an `Eq`: a window frame is four `f64`s, so they stay `PartialEq`-only.

## The counter on the root

`crates/mercury/src/state/mod.rs`, the `Mercury` struct gains a field:

```rust
    /// The source of timer ids. On the root because the program mints these; every arm draws the
    /// next from here, so an id is a function of state. See `CLAUDE.md`'s ambient-state rule.
    timer_ids: TimerIds,
```

`TimerIds` derives `Default`, so `Mercury`'s construction is unchanged where it is `..Default::default()`; a hand-written constructor sets `timer_ids: TimerIds::default()`.

## The arming sites

Every arming site holds `&mut` to the root or is handed `&mut TimerIds`, mints, and stores the owned guard on its own state. The guard borrows nothing, so the `&mut TimerIds` borrow ends at the `timer_effect_and_guard` call, before the store.

`toggle_overlay` (`mod.rs:619`), a root method, before:

```rust
        let (guard, effect) =
            timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
        self.overlay = Some(guard);
```

after:

```rust
        let (guard, effect) = timer_effect_and_guard(&mut self.timer_ids, OVERLAY_DWELL);
        self.overlay = Some(guard);
```

`Windows::asking_for` (`mod.rs:343`) is on the `Windows` sub-struct, so it takes the source and its caller `Windows::placing` forwards it. before:

```rust
    fn asking_for(&mut self, target: WindowFrame) -> Vec<MercuryEffect> {
        let (timer, effect) =
            timer_effect_and_guard(PLACEMENT_SETTLE, |id| MercuryEvent::Timer(TimerFired(id)));
```

after:

```rust
    fn asking_for(&mut self, timer_ids: &mut TimerIds, target: WindowFrame) -> Vec<MercuryEffect> {
        let (timer, effect) = timer_effect_and_guard(timer_ids, PLACEMENT_SETTLE);
```

Its caller passes `&mut root.timer_ids` beside the `&mut root.windows` receiver: `root.windows.placing(&mut root.timer_ids, target)`. Different fields of `Mercury`, so the two mutable borrows do not collide.

The layer constructors that arm a return-home timer take the source. `NavLayer::new` (`state/nav.rs:39`) is representative; `AppLayer::new`, `SiteLayer::new`, and `ResizeLayer::new` are the same shape. before:

```rust
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home();
```

after:

```rust
    pub(crate) fn new(timer_ids: &mut TimerIds) -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home(timer_ids);
```

The transition handlers ascend to the root for `set_layer` and pass `&mut root.timer_ids` to the constructor. `handlers/home.rs`'s nav transition is representative, before:

```rust
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend_mut().set_layer(nav);
```

after:

```rust
    let root = node.parent.ascend_mut();
    let (nav, timer) = NavLayer::new(&mut root.timer_ids);
    let mut effects = root.set_layer(nav);
```

`&mut root.timer_ids` and the layer field `set_layer` writes are disjoint fields of `Mercury`, and the borrow for `new` is released before `set_layer` reborrows `root`. The guard is born inside the fresh `nav`, which `set_layer` moves into `root.layer`, so nothing stores onto the leaf the handler ascended from.

`arm_return_home` stays (six callers, the one place that names the return-home delay); `arm_jk_timeout` is inlined at its one caller. before:

```rust
fn arm_return_home() -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(RETURN_TO_HOME_TIMEOUT, |id| {
        MercuryEvent::Timer(TimerFired(id))
    });
    (guard, MercuryEffect::Timer(effect))
}

pub(crate) fn arm_jk_timeout(window: Duration) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(window, |id| MercuryEvent::Timer(TimerFired(id)));
    (guard, MercuryEffect::Timer(effect))
}
```

after (`arm_jk_timeout` deleted):

```rust
fn arm_return_home(timer_ids: &mut TimerIds) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(timer_ids, RETURN_TO_HOME_TIMEOUT);
    (guard, MercuryEffect::Timer(effect))
}
```

`arm_jk_timeout`'s one caller, `crates/mercury/src/handlers/root.rs:41`, before:

```rust
                let (guard, timer) = arm_jk_timeout(window);
                root.typing_state.jk.hold(guard);
                vec![timer]
```

after:

```rust
                let (guard, effect) = timer_effect_and_guard(&mut root.timer_ids, window);
                root.typing_state.jk.hold(guard);
                vec![MercuryEffect::Timer(effect)]
```

`&mut root.timer_ids` is released at the call, and `root.typing_state` is a different field, so the store on the next line does not collide. Every call to a `*Layer::new()` is updated to pass the source: the `home.rs` transitions and the `handlers/nav.rs` in-app transition. `Windows::placing` forwards its own `&mut TimerIds` to `asking_for`.

## The tests

`crates/mercury/tests/transitions.rs` builds expected timer effects and now needs a source to mint from. The helpers at `transitions.rs:26,33,1209,1448` gain a `&mut TimerIds` argument threaded from the test's expected state and drop the `fired` closure, wrapping the `TimerEffect` with `MercuryEffect::Timer` as the code under test does, so the id in the expected effect matches the id the transition minted. The `fired` free function (`transitions.rs:39`) goes with the closure.

`timer_id(effects)` (`transitions.rs:44`), which reads the id off a produced effect, is unaffected: it reads whatever the transition minted, and that is now a function of the state the test dispatched against.
