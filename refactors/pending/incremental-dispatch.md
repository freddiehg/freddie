# incremental dispatch

What a sample-rate event source would cost per event, whether incremental computation (isograph's pico) is the tool that keeps it cheap, and the rule that does. The standing example is the pointer: mouse-mode.md leaves open whether the pointer's position becomes model state fed by an event stream, and that stream fires at input rate for every physical twitch, hundreds of events a second against a dispatch built for keystrokes.

## what one dispatch costs

The derive emits, per node on the active path, one `opt` per bind row, snapped before the descent:

```rust
// what #[derive(Bind)] emits per row (bind_macro, `opt`)
let opt_0 = match ::core::convert::TryFrom::try_from(event) {
    Ok(ev) => {
        let trigger = /* the row's value, or its closure called with the state */;
        if ::bind::EventTrigger::is_matching(&trigger, ev) {
            Some((ev, snap))
        } else {
            None
        }
    }
    Err(_) => None,
};
```

So per event, the work is:

- The descent along the active path: enum matches and pointer chasing, no allocation.
- Per row on the path: the `TryFrom` narrow of the unified event, one enum branch. Rows for other event types fall out here, before their trigger is evaluated, so a pointer event does not run the key rows' closures.
- Per row whose event type matches: the trigger — a value compares, a closure runs against the state.
- Per `#[derived_child(f)]` edge on the path: `f` runs and builds the level's data, every dispatch, whatever the event.
- The effects: an unmatched dispatch returns `Vec::new()`, which does not allocate.
- The dispatch record: `info!(event = ?event, effects = ?effects, duration_us, state = ?state, "dispatch")`, the whole model under `Debug`, serialized to JSON, written to the file — which records down to `debug` whatever the terminal shows.

Everything above the record is nanoseconds to single-digit microseconds. The record is the cost: at pointer rate it is multiple kilobytes of state debug per sample, hundreds of times a second, forever, into a file that structured-log.md promises keeps everything.

## nothing is cached, so nothing goes stale

There is no realized bind table at runtime. The check's `accumulate` set exists only under the `check` feature, in tests. Triggers are evaluated fresh on every dispatch, closure triggers read the state as it stands on the way down, and a derived level's data is built by its function during the descent and dropped when the dispatch ends. A state write cannot strand a stale binding, because no binding outlives one dispatch.

So incremental computation has no correctness work to do here. The only question it could answer is recomputation cost, and the recomputation is already near-free. What is not free scales with event rate: the record, and any derived computation that stops being a field read.

## pico

pico (isograph's incremental engine) is: sources set into a `Database` (`db.set(source)` hashes and stores; `db.get(id)` reads), and `#[memo]` functions that are pure over `&Db`, memoized per parameter set, with dependencies tracked on a runtime stack, revisions verified per epoch, and backdating — a re-run whose value is `Eq` to the old one keeps downstream memos valid. Storage is `DashMap`, `boxcar::Vec`, an `LruCache` of top-level calls, and a garbage collector.

It does not fit dispatch:

- Ownership. The model is a single-owner `&mut` tree; handlers mutate in place through paths. pico owns its values: state would live in the database, read as queries, written as `set` calls, `Clone + Hash + 'static` throughout. That is a rewrite of laserbeam's premise, not an addition to it.
- Keys. Memoization is per parameter set, and dispatch's parameter is the event. A sample-rate event's payload differs every time, so every call is a miss; memoizing dispatch caches nothing.
- Machinery. `DashMap`, `LruCache`, interior mutability behind `&Db`, a GC: the shelf of primitives the shared-state rule exists to question, justified in a compiler running parallel queries over a long-lived database, unjustified in a one-thread model whose whole state is one struct.
- Dynamic tracking answers a static question. pico discovers what a computation read by running it; a handler here declares what it reads in its bounds and its node.

The piece of pico that is right for this problem is backdating: recompute the cheap upstream thing, compare with `Eq`, and let everything downstream stand when it is equal. That piece does not need the database, and the rest of this doc is it.

## the rule: sources dispatch transitions, not samples

An event is a transition of a value some binding can react to, never a sample of a raw signal. The sources already hold this line — the extension sends a tab event only when the URL differs from the last one sent, app-nav reports foreground changes, displays reports topology changes — and a pointer source holds it the same way: the raw position is the sample; the value a binding can mean is the reduction.

```rust
/// Where the pointer is, reduced to what a binding can mean.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    TopLeft,
    Elsewhere,
}

/// A region transition, carrying where the pointer now is.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct RegionChanged {
    pub region: Region,
}

/// A trigger matching any region transition.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Regioned;

impl EventTrigger for Regioned {
    type Event = RegionChanged;
    fn is_matching(&self, _ev: &RegionChanged) -> bool {
        true
    }
}
```

The reduction has two possible homes, decided per source.

### reduction in the source (the default)

The source thread owns the raw signal, computes the reduced value per sample, and sends an event only on a transition — `pushUrl`'s `lastSent` dedupe, in Rust:

```rust
// the pointer source's callback, on its own thread
let region = Region::of(sample, screen);
if last_sent != Some(region) {
    last_sent = Some(region);
    // A closed channel means the event loop has ended, which is the way out running.
    let _ = event_tx.send(MercuryEvent::Region(RegionChanged { region }));
}
```

The model never sees the sample rate. Dispatch cost, record included, is per transition, and a transition is rare by construction. If a handler needs the current region, the root mirrors it from the event exactly as `foreground` mirrors the front app; the raw position never enters the model at all.

This is the default because it needs nothing new: no dispatch change, no record change, no state. Its limit is that the reduction can read only what the source holds, so a reduction over model state (regions cut against window frames the model tracks) cannot live here.

### reduction at the gate (when it reads the model)

When the reduction needs model state, the sample has to reach the model, and the short circuit moves into `handle`'s pre-bind section, beside the gates that already live there:

```rust
// state/mod.rs
pub fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> {
    if let MercuryEvent::PointerSample(sample) = event {
        let before = self.pointer.region(&self.windows);
        self.pointer.record(*sample);
        let after = self.pointer.region(&self.windows);
        if after == before {
            return Vec::new();
        }
        return bind::dispatch::<MercuryStruct, Self, _>(
            self,
            &MercuryEvent::Region(RegionChanged { region: after }),
        );
    }
    // …existing pre-bind gates…
    bind::dispatch::<MercuryStruct, Self, _>(self, event)
}
```

```rust
/// The pointer as the model holds it: the raw signal, and the reduction bindings see.
#[derive(Debug)]
pub struct Pointer {
    position: Point,
}

impl Pointer {
    /// Record one sample. Assigns, never accumulates, so a replayed sample lands where it did.
    pub fn record(&mut self, sample: Point) {
        self.position = sample;
    }

    /// The reduction: which region the pointer is in, cut against what the model knows.
    #[must_use]
    pub fn region(&self, windows: &Windows) -> Region {
        // pure over its arguments
    }
}
```

The bind walk runs only on transitions; a swallowed sample costs the record write and two reductions. Dispatch stays a pure function of `(state, event)`: the sample is a mirror write, the transition is derived from state, and a replay reproduces both.

### the record at sample rate

The gate placement still passes every sample through `dispatch_event`, and the full record per sample is the one cost that matters. So the record's detail becomes a property of the event:

```rust
/// How one dispatch is recorded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordDetail {
    /// The full record: event, effects, duration, state.
    Full,
    /// One `trace` line, no state: for sample-rate events whose dispatch usually reduces to
    /// nothing. The transitions they produce dispatch as their own events and are recorded in
    /// full, so the story of a run stays in the file; the samples between transitions do not.
    Sample,
}

impl MercuryEvent {
    #[must_use]
    pub const fn record_detail(&self) -> RecordDetail {
        match self {
            Self::PointerSample(_) => RecordDetail::Sample,
            _ => RecordDetail::Full,
        }
    }
}
```

`dispatch_event`, before:

```rust
    let effects = state.handle(event);
    let duration_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    info!(event = ?event, effects = ?effects, duration_us, state = ?state, "dispatch");
```

After:

```rust
    let effects = state.handle(event);
    let duration_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    match event.record_detail() {
        RecordDetail::Full => {
            info!(event = ?event, effects = ?effects, duration_us, state = ?state, "dispatch");
        }
        RecordDetail::Sample => trace!(event = ?event, effects = ?effects, duration_us, "dispatch"),
    }
```

This narrows the one-record-per-dispatch promise deliberately: it holds in full for every event that can change what the model does, and sample events buy out of it because their information content is the transitions, which are recorded. `trace` sits below the file's `debug` floor, so samples cost nothing on disk unless a debugging session asks for them.

## an expensive derived value, if one appears

Derived children and closure triggers recompute per dispatch, and today every one is a field read or a small enum build. If one ever gets expensive (label enumeration over an AX tree, a layout solve), the fix is pico's cutoff without pico: a cache owned by the node that reads it, keyed by an `Eq` fingerprint of exactly what the computation reads.

```rust
// crates/freddie/src/lib.rs
/// A derived value recomputed only when its inputs change.
///
/// Model state: the node that owns one carries it through every dispatch, and the fingerprint
/// must name everything the computation reads, or a stale value survives an input it missed.
#[derive(Debug)]
pub struct Cached<I: PartialEq, V> {
    input: I,
    value: V,
}

impl<I: PartialEq, V> Cached<I, V> {
    /// The cache primed with its first computation.
    pub fn new(input: I, compute: impl FnOnce(&I) -> V) -> Self {
        let value = compute(&input);
        Self { input, value }
    }

    /// The value for `input`, recomputed only when `input` differs from the last call's.
    pub fn get(&mut self, input: I, compute: impl FnOnce(&I) -> V) -> &V {
        if self.input != input {
            self.value = compute(&input);
            self.input = input;
        }
        &self.value
    }
}
```

This is state on the model, with the obligations state carries, which is why it is adopted per computation, on measurement, and not as ambient infrastructure.

## changes

1. Now: this rule, binding on every future source. A source that can dispatch at sample rate reduces before it sends, in the source by default, at the gate when the reduction reads the model. No code changes: no sample-rate source exists yet, and mouse-mode.md's stage-one timers are not one (each tick carries real work, a `MoveBy`, and the timer dies with the keyup).
2. With the first sample-rate source: `RecordDetail`, the `record_detail` method with that source's variant as its first `Sample` arm, and the `dispatch_event` match, as written above. If the source's reduction reads the model, the gate in `handle` and the mirror state, as written above; otherwise the source-side dedupe and no model change.
3. If a derived computation is measured expensive: `Cached` in `crates/freddie`, owned by the node whose computation it caches.
