# log unhandled events at the root

Every event that nothing on the active path claims falls through to a root catch-all that logs it. The catch-all is an ordinary exclusive bind: `Any => log_unhandled`, last on the root so every specific bind (including `AnyKey`) claims first. Dispatch already reports whether the claim was taken; this makes the miss visible in the log file instead of dropping into silence.

What reaches it today is a timer firing whose guard is gone (stale, cancelled, or never held). Keys never reach it: `AnyKey => maybe_pass_through` is earlier on the same node and claims every key. Foreground, tab, window, and quit each have their own root bind and claim first. A new source with no bind lands here until one is written.

## `Any`

A trigger whose source event is the unified event and whose match is always true. Narrowing is the identity `TryFrom<&MercuryEvent> for &MercuryEvent` (std's `TryFrom<T> for T`); `is_matching` never rejects.

`crates/mercury/src/sources.rs`, after `AnyKey`:

```rust
/// A trigger matching every event, whichever source it came from.
///
/// Bound last at the root as the last resort: exclusive dispatch is leafward, then rootward in
/// source order, so a bind earlier on the same node (or deeper in the tree) claims first. What
/// reaches this is an event nothing more specific wanted.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Any;

impl EventTrigger for Any {
    type Event = crate::MercuryEvent;
    fn is_matching(&self, _ev: &crate::MercuryEvent) -> bool {
        true
    }
}
```

`Event = MercuryEvent` rather than a per-source type, because the point is to match across sources. The handler is handed the unified event, which is what the log wants.

## `MercuryTrigger`

`crates/mercury/src/model.rs`, before:

```rust
#[derive(Clone, PartialEq, Eq, Hash, Debug, derive_more::From)]
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    KeyChord(KeyChord),
    AnyKey(AnyKey),
    Foregrounded(Foregrounded),
    Tabbed(Tabbed),
    Windowed(Windowed),
    Quit(Quit),
}
```

after:

```rust
#[derive(Clone, PartialEq, Eq, Hash, Debug, derive_more::From)]
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    KeyChord(KeyChord),
    AnyKey(AnyKey),
    Any(Any),
    Foregrounded(Foregrounded),
    Tabbed(Tabbed),
    Windowed(Windowed),
    Quit(Quit),
}
```

`use crate::{Any, AnyKey, ...}` gains `Any`. The check feature's accumulate set holds `MercuryTrigger::Any(Any)` for the root bind; it collides with nothing, because equality is by variant and payload and no other trigger is `Any`.

## Root bind

`crates/mercury/src/state/mod.rs`, the root's `#[bind(..)]`, before:

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Windowed => record_windows,
    Quit => quit,
    // Only this run's window: a firing from a run that has since ended matches nothing, so the
    // handler never sees it.
    |mercury_path| mercury_path.typing_state.jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    // Only the showing that is up: a dwell from one already replaced matches nothing.
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    // Only the placement still outstanding: a firing from one already landed matches nothing.
    |mercury_path| mercury_path.windows.pending_timer().map(TimerGuard::trigger) => placement_settled,
    AnyKey => maybe_pass_through,
)]
```

after:

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Windowed => record_windows,
    Quit => quit,
    // Only this run's window: a firing from a run that has since ended matches nothing, so the
    // handler never sees it.
    |mercury_path| mercury_path.typing_state.jk.window_timer().map(TimerGuard::trigger) => jk_timeout,
    // Only the showing that is up: a dwell from one already replaced matches nothing.
    |mercury_path| mercury_path.overlay_timer().map(TimerGuard::trigger) => hide_overlay,
    // Only the placement still outstanding: a firing from one already landed matches nothing.
    |mercury_path| mercury_path.windows.pending_timer().map(TimerGuard::trigger) => placement_settled,
    AnyKey => maybe_pass_through,
    // Last: every earlier bind on this node, and every deeper bind, has already had its turn.
    // Claims the miss so the claim bit is set, and writes one record naming the event.
    Any => log_unhandled,
)]
```

`use crate::{..., AnyKey, ...}` gains `Any`. Source order is the claim order at this node: `Any` after `AnyKey` is what keeps keys in `maybe_pass_through`.

## Handler

`crates/mercury/src/handlers/root.rs`, after `maybe_pass_through`:

```rust
/// An event nothing on the active path claimed.
///
/// Effects none: the only work is the record. `debug` so a quiet terminal stays quiet and the
/// file (always at `debug`) keeps every miss. The dispatch record still writes at `info` for
/// the same event; this line is the one that says the claim was the root's last resort.
pub(crate) fn log_unhandled(
    ev: &MercuryEvent,
    _node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    debug!(event = ?ev, "unhandled");
    Vec::new()
}
```

The module gains `use tracing::debug;` and `use crate::MercuryEvent;`. The event is borrowed the way every other exclusive handler takes its source event today.

Generated dispatch at the root for this bind (same shape as every other exclusive; shown so the identity narrow is explicit):

```rust
// inside Mercury's Dispatch::dispatch, after the AnyKey check:
if let ::core::option::Option::Some(ev) =
    ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
{
    let trigger = Any;
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        if let ::core::option::Option::Some(()) = claim.try_take() {
            *effs = ::core::iter::Iterator::collect(
                ::core::iter::IntoIterator::into_iter(log_unhandled(ev, node)),
            );
            return ::core::option::Option::None;
        }
    }
}
```

`try_from(event)` here is `TryFrom<&MercuryEvent> for &MercuryEvent`, always `Ok(event)`. `is_matching` is always true. The only gate that matters is `claim.try_take()`: if anything earlier on the path already claimed, this arm is a no-op.

## Exports

`crates/mercury/src/lib.rs`:

```rust
pub use sources::{
    Any, AnyKey, App, ForegroundEvent, Foregrounded, Quit, Site, TabEvent, Tabbed, WindowEvent,
    Windowed, host,
};
```

## Tests

A stale or unmatched timer firing used to return `(vec![], false)`: no bind claimed. With `Any` last, the same firing returns `(vec![], true)`: effects still empty, claim taken by `log_unhandled`. The layer and every other field stay put; only the handled bit flips.

`crates/mercury/tests/transitions.rs`, each of these:

```rust
// a_firing_from_a_layer_already_left_matches_nothing
assert_eq!(
    m.handle(&fired(first)),
    (vec![], false),
    "no binding matches a stale firing"
);

// a_firing_in_a_layer_that_set_no_timer_matches_nothing
assert_eq!(m.handle(&fired(stale)), (vec![], false));

// a_firing_from_a_run_that_ended_matches_nothing
assert_eq!(
    m.handle(&fired(first)),
    (vec![], false),
    "no binding matches a stale firing"
);

// a_firing_with_no_run_in_progress_matches_nothing
assert_eq!(m.handle(&fired(stale)), (vec![], false));

// the_overlay_hides_after_the_dwell (second firing)
assert_eq!(m.handle(&fired(timer_id(&shown))), (vec![], false));

// a_dwell_from_a_showing_already_gone_matches_nothing
assert_eq!(m.handle(&fired(first)), (vec![], false));
```

become:

```rust
// a_firing_from_a_layer_already_left_matches_nothing
assert_eq!(
    m.handle(&fired(first)),
    (vec![], true),
    "stale firing reaches the root Any and is claimed with no effects"
);

// a_firing_in_a_layer_that_set_no_timer_matches_nothing
assert_eq!(m.handle(&fired(stale)), (vec![], true));

// a_firing_from_a_run_that_ended_matches_nothing
assert_eq!(
    m.handle(&fired(first)),
    (vec![], true),
    "stale firing reaches the root Any and is claimed with no effects"
);

// a_firing_with_no_run_in_progress_matches_nothing
assert_eq!(m.handle(&fired(stale)), (vec![], true));

// the_overlay_hides_after_the_dwell (second firing)
assert_eq!(m.handle(&fired(timer_id(&shown))), (vec![], true));

// a_dwell_from_a_showing_already_gone_matches_nothing
assert_eq!(m.handle(&fired(first)), (vec![], true));
```

The assertions that the live timer, live run, and live overlay are untouched stay. The message on the stale-firing cases changes with the bit: "matches nothing" is no longer true of the claim, only of every bind that would have done work.

No new test for the log line itself: the file subscriber is the daemon's, and the model tests drive `handle` with no tracing. The claim bit and the empty effects are what the table can assert.

## End-user

An orphaned timer firing (or any future unbound event) still dispatches. The dispatch record at `info` is unchanged. One extra `debug` record appears with `message = "unhandled"` and the event under `event`. `mercury logs --level debug` shows it; the default terminal filter does not. Keys, foreground, tab, window, and quit never produce it while their root binds remain.
