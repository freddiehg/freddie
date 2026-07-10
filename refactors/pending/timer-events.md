# timer events

A timer is a source, not an effect. It fires an event, that event dispatches like any other, and a handler mutates state through the path it holds.

Saying it the other way round is the mistake worth avoiding. If scheduling were an effect, a handler would return `ScheduleTimer { after, id }`, something would have to remember the id, and a `TimerFired(id)` event would have to be correlated back to the state that asked for it. Every one of those pieces exists only because the timer was modeled in the wrong direction.

## A timer is a trigger

The design already says this, in what is now `refactors/past/overall-plan.md`:

> Timers are modeled as explicit states. The pattern: we enter a state, we schedule a timer for 500ms, and after 500ms we handle that as just another event. Before-event and after-event are two distinct states. There is no hidden tap-vs-hold primitive.

Which means a timer belongs in the `Trigger` enum, next to `Key` and `Foregrounded`, and a node declares it the way it declares a key:

```rust
#[bind(
    Key::Escape.down() => to_home,
    After(Duration::from_millis(500)) => on_timeout,
)]
struct SomeLayer {}
```

Entering the state arms the timer. Leaving disarms it. Nothing schedules anything, nothing carries an id, and there is no way to leak a timer belonging to a state you are no longer in, because arming and disarming fall out of the same accumulation diff that registers a key with the OS.

That is the mechanism `overall-plan.md` described and nothing has yet used:

> One outer handler owns registration. It receives the accumulated `Trigger` diff and routes each variant to its OS mechanism.

`bind::accumulate` exists and returns the active trigger set. Nothing consumes it. The keyboard does not need it, because the tap is global and mercury dispatches every key. A timer is the first trigger that genuinely needs the diff, since arming is an action that must happen on entry and disarming on exit. So the first timer is also the thing that forces the registration half of `bind` to be real.

## What mercury does today

There is exactly one timer, and it cheats. `spawn_killswitch` sleeps and then sends `MercuryEffect::Kill` straight into the effect channel, bypassing the model. The state tree never sees it. That is fine for a dev safety net, and it is not the pattern.

## What wants timers

Tap versus hold. Explicitly not a primitive. `escape` tapped goes home; `escape` held enters a layer. Two states and a timer between them.

Keyboard-mouse mode. Continuous pointer motion needs a repeating timer feeding events while a key is held, not one event per keypress. This is the case that makes repeat, versus one-shot, a real question rather than a nicety.

Auto-hiding an overlay. voicemode's `showBrief(layer)` flashes the layer name and clears it. That is a state with a timer.

Debouncing. Rapid app switches and display reconfiguration both produce bursts; both `foreground-events.md` and `display-events.md` name debouncing as open. A timer trigger is how you would express it, and doing so would keep the debounce in the model rather than in the source crates.

## A timer needs an identity, and `After(500ms)` does not have one

Triggers are compared by value, and `accumulate` returns a `HashSet<Trigger>`. So `After(500ms)` bound on state A and `After(500ms)` bound on state B are the same trigger. Three things break at once.

A transition from A to B produces no difference in the accumulated set, so B's timer never arms. It inherits A's, already partly elapsed.

Two active nodes on one path both binding `After(500ms)` is a `DuplicateTrigger` error out of `insert_or_error`, even though they plainly mean two different timers.

Re-entering the same state, which mercury does every time `escape` is pressed in `Home`, produces no difference either, so nothing restarts.

So a timer's identity has to include where it was bound, not only how long it is. That is not a wrinkle in the trigger enum; it changes what a `Trigger` is and therefore what `accumulate` returns. Keys do not have this problem, because a key bound at two places on the active path genuinely is a conflict, which is why `insert_or_error` is right for them and wrong here.

The "restart or leave running" question below is downstream of this one. It cannot be asked until a trigger can tell two timers apart.

## Open questions

- One-shot and repeat as different triggers, or one trigger with a repeat flag?
- How does a timer trigger carry the identity of the node that bound it, and what does that do to `accumulate`'s `HashSet<Trigger>` and to `insert_or_error`?
- Once it can: re-entering a state that is already armed, restart the timer or leave it running? Restart is the obvious answer and it is not obviously right for a repeat.
- Where the timer lives. `tokio::time` on the worker thread is already there and needs nothing new. A `CFRunLoopTimer` on main would also work and would deliver where AppKit callbacks deliver. tokio is the answer unless something wants the main run loop, and nothing does.
- Interaction with `prioritization.md`. A fast repeat timer feeding the same queue as the keyboard can starve typing, which is exactly the case that doc was written for.
- Does arming a timer belong to the accumulation diff, or does entering a state explicitly arm it? The diff is the design, and it is more machinery than a first timer needs. The temptation to special-case this will be strong, and taking it means the diff never gets built.
