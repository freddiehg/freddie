# timing every event

Not built. Measure the latency of every dispatched event, from receiving it to producing and performing its effects, and make slow ones obvious.

## What to measure

The path of one key:

1. The tap captures a key, on the tap thread, in `freddie_keyboard`'s callback.
2. It goes on the event channel.
3. The event loop, on the worker, dispatches it: `state.handle(event)` produces effects. Pure, and fast (microseconds).
4. The effects go on the effect channel.
5. The effect loop performs each: `emit` (microseconds), `foreground` (spawns a thread), `place` (Accessibility, tens of milliseconds).

Two things are worth timing, and they answer different questions.

- Per-event cost: from the event loop receiving an event to finishing its dispatch, and from the effect loop receiving an effect to finishing it. This says where the work is. Dispatch is microseconds; effects vary by kind.
- End-to-end lag: from the tap capturing a key to the remapped key being emitted. This is what the user feels, and it spans both channels and both loops, so it catches queueing and scheduling that per-hop timing misses.

## Instrumentation

The dispatch log already writes one line per event (the event, its effects, the resulting state). Add the elapsed time to it: an `Instant` around `dispatch_event`, recorded as a `dispatch_us` field on that same line. Do the same in `perform_effect`, recording an `effect_us` next to each performed effect. That keeps the "one line tells the whole story of one event" shape and adds the cost.

For the end-to-end lag, stamp the event with the `Instant` it was captured (in the tap callback) and carry it on the `MercuryEvent`. When an effect for that event is emitted, log `now - captured`. That is the number that matters for perceived responsiveness; the per-hop numbers explain it.

Tracing shape: record the elapsed as a field on the existing per-event record rather than reaching for span-timing machinery. A manual `Instant` and a `dispatch_us` field is simpler than `tracing-timing` or `with_span_events`, and it fits the one-line-per-event format the log already uses. Spans are the alternative if we later want nested timing (dispatch within end-to-end within a batch), but flat fields are enough to start.

## Surfacing slow ones

tracing does not color by duration; the record's LEVEL drives color in the terminal fmt layer. So map slowness onto level: log an event at `warn` when its dispatch or an effect exceeds a threshold, at `info` otherwise. The terminal colors `warn` yellow, so a slow event stands out with no custom renderer, and the log file (always at `debug`) keeps every timing regardless of level. Per `freddie/CLAUDE.md`, `LOG_LEVEL` already gates only the terminal, so a quiet terminal still shows the slow ones if the threshold bumps them to `warn`.

That level-based highlight is the pragmatic reading of "render slow ones in a different color." A custom tracing layer could color by exact duration (a gradient), but it needs a bespoke fmt layer; the level bump needs nothing new.

## Thresholds

The threshold has to be per effect kind, because the kinds have different floors. `emit` and a bare dispatch are microseconds, so a millisecond is already suspicious. `place` is inherently tens of milliseconds (the Accessibility round-trip), so its threshold is higher and a warn there means "even slower than the expected slow." Pick a threshold per effect type rather than one global number.

## Open questions

- Whether to carry the capture `Instant` on the event for true end-to-end latency, or only time the two loops separately. End-to-end is the honest number but threads an `Instant` through the event type; per-hop is simpler and still finds the expensive effect.
- The exact thresholds per effect kind for the warn bump.
- Whether to aggregate over a run (min, max, p99) as well as per-event lines. Per-event first; aggregation only if a pattern needs hunting across many events.
- Whether the tap-thread capture timestamp and the worker-thread emit timestamp share a clock cleanly (`Instant` is monotonic and process-wide, so yes).
