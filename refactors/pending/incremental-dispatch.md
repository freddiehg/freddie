# incremental dispatch

This doc answers a question about event volume. A source like the pointer produces samples at input rate, hundreds per second while the mouse moves, and mouse-mode.md leaves open whether the pointer's position should become model state fed by an event stream. If it does, every sample runs through a dispatch that was built for keystrokes. The worry is that re-executing the full bind walk per sample is wasteful, and the candidate remedy is incremental computation in the style of isograph's pico. So the doc does three things: it accounts for where a dispatch actually spends its time, it evaluates pico against that accounting, and it states the rule freddie adopts instead, along with the mechanisms that implement the rule when a sample-rate source arrives.

## what one dispatch costs

The place to start is what the derive generates, because the cost of "re-executing the full bind" is the cost of this code. For every node on the active path, the derive emits one `opt` per bind row, and each `opt` is evaluated before the descent:

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

Walking through what one event pays:

- The descent along the active path consists of enum matches and pointer chasing, and it allocates nothing.
- Each row on the path first narrows the unified event with `TryFrom`, which is a single match on the event's variant. A row whose source event is a different type falls out here, before its trigger is even constructed. A pointer event would therefore never run the key rows' closures; those rows cost one failed enum match each.
- Each row whose event type does match then evaluates its trigger. A value trigger compares against the event, and a closure trigger runs against the state as it stands on the way down.
- Each `#[derived_child(f)]` edge on the path calls `f` and builds that level's data. This happens on every dispatch, whatever the event is, because the descent has to pass through the level.
- The effects vector for a dispatch that matched nothing is `Vec::new()`, which does not allocate.
- Finally, the daemon writes the dispatch record: `info!(event = ?event, effects = ?effects, duration_us, state = ?state, "dispatch")`. That line formats the entire model under `Debug`, serializes the result to JSON, and appends it to the log file, which records down to `debug` no matter what the terminal shows.

Everything above the record adds up to nanoseconds, or single-digit microseconds when a derived child does real construction. The record is in a different class entirely. At pointer rate it means multiple kilobytes of state debug per sample, hundreds of times a second, for as long as the mouse moves, into a file that structured-log.md promises keeps everything. If sample-rate events ever flow through dispatch unmodified, the log is what breaks first, not the bind walk.

## nothing is cached, so nothing goes stale

One of the original worries was that the binds themselves might change as a consequence of a state write, which would make any cached bind structure a staleness hazard. The answer is that no such structure exists. There is no realized bind table at runtime; the trigger set that `accumulate` collects exists only under the `check` feature, which ships in tests and nowhere else. Triggers are evaluated fresh on every dispatch, closure triggers read the state as it stands during the descent, and a derived level's data is built by its function on the way down and dropped when the dispatch ends. A state write cannot strand a stale binding, because no binding outlives the dispatch it was evaluated in.

This settles the correctness half of the incrementality question before it opens. An incremental engine earns its complexity by tracking which cached results a write invalidated, and freddie caches nothing, so there is nothing to invalidate. The only question left for incrementality to answer is recomputation cost, and the accounting above says the recomputation is already near-free. What actually scales with event rate is the record, plus any derived computation that someday stops being a field read.

## pico

pico is isograph's incremental computation engine. Its shape is a `Database` holding sources (`db.set(source)` hashes the source's key and stores the value; `db.get(id)` reads it back) together with `#[memo]` functions that are pure over `&Db`. A memoized call is keyed by its parameter set. While it runs, the database records every source and every other memo it reads on a runtime dependency stack. On a later call, pico checks whether any recorded dependency changed since the value was last verified, using epochs, and reuses the stored value when none did. When a dependency did change, it re-runs the function, and if the new value compares equal to the old one it backdates the node, meaning downstream memos that depended on it remain valid without re-running. The storage behind all of this is a `DashMap`, a `boxcar::Vec`, an `LruCache` of top-level calls, and a garbage collector over the lot.

Measured against freddie's dispatch, pico does not fit, for reasons that go to structure rather than tuning.

The first is ownership. The model is a single-owner mutable tree, and handlers mutate it in place through typed paths. pico owns the values it computes over: state would have to live in the database, be read through queries, be written through `set` calls, and satisfy `Clone + Hash + 'static` throughout. Adopting it would amount to rewriting laserbeam's premise rather than adding a layer on top of it.

The second is the memo key. A memoized function is cached per parameter set, and dispatch's parameter is the event. A sample-rate event carries a payload that differs on every sample, so every call would be a cache miss. Memoizing dispatch would cache nothing at all, which removes the entire benefit before any cost is paid.

The third is the machinery itself. Interior mutability behind `&Db`, concurrent maps, an LRU, a garbage collector: this is the shelf of primitives the shared-state rule tells us to question every time. A compiler running parallel queries over a database that lives for hours justifies them. A one-thread model whose entire state is one struct does not.

The fourth is that dynamic dependency tracking answers a question freddie already answers statically. pico discovers what a computation read by watching it run. A handler here declares what it reads in its bounds and in the node it binds on, and the compiler checks the declaration.

One piece of pico survives this assessment, and it is the piece the rest of this doc builds on: backdating. Recompute the cheap upstream value, compare it with `Eq`, and let everything downstream stand when the comparison says nothing changed. That idea does not need the database that pico wraps around it.

## the rule: an event is a transition

The rule freddie adopts is about what qualifies as an event. An event reports that a value some binding can react to has taken a new state. A raw signal's samples do not qualify, because between transitions they carry no information a binding can act on. The existing sources already hold this line without naming it: the extension sends a tab event only when the URL differs from the last one it sent, app-nav reports the foreground only when it changes, and displays reports topology per change rather than per query. A pointer source holds the same line by reducing the raw position to the value bindings actually mean, and reporting that value's transitions.

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

The reduction needs a home, and there are two candidates. Which one a source uses is decided by what the reduction needs to read.

### reduction in the source, which is the default

When the reduction is a function of the raw signal alone (plus configuration the source can own, like the screen's dimensions), it lives in the source. The source thread computes the reduced value on every sample and sends an event only when the value differs from the last one it sent. This is `pushUrl`'s `lastSent` dedupe, translated into Rust:

```rust
// the pointer source's callback, on its own thread
let region = Region::of(sample, screen);
if last_sent != Some(region) {
    last_sent = Some(region);
    // A closed channel means the event loop has ended, which is the way out running.
    let _ = event_tx.send(MercuryEvent::Region(RegionChanged { region }));
}
```

Under this placement the model never sees the sample rate at all. Dispatch cost, the record included, is paid per transition, and transitions are rare by construction, since the pointer crosses a region boundary far less often than it moves. If a handler needs to know the current region, the root mirrors it from the event exactly the way `foreground` mirrors the front app, and the raw position never enters the model.

This placement is the default because it requires nothing new. There is no dispatch change, no record change, and no model state. Its limit is that the reduction can read only what the source holds. A reduction defined over model state, say regions cut against the window frames the model tracks, cannot be computed on the source thread, because the source has no access to the model.

### reduction at the gate, when it reads the model

When the reduction needs model state, the samples have to reach the model, and the short circuit moves into `handle`'s pre-bind section, beside the gates that already live there. The sample event's handler records the position, computes the reduction before and after the write, and enters the bind walk only when the reduced value moved:

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

Under this placement the bind walk runs only on transitions, and a swallowed sample costs two reduction computations and the record write. Purity survives: the sample is an idempotent mirror write, the transition event is derived from state the model holds, and replaying the same `(state, event)` pair reproduces both the write and the dispatch that followed it.

### the record at sample rate

The gate placement still passes every sample through `dispatch_event`, and the accounting section established that the full record per sample is the one cost that matters. So the record's level of detail becomes a property of the event:

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

This narrows the one-record-per-dispatch promise, and it does so deliberately. The promise holds in full for every event that can change what the model does. Sample events buy out of it because their information content lives in the transitions they produce, and the transitions dispatch as their own events with full records. `trace` sits below the file's `debug` floor, so the samples cost nothing on disk unless a debugging session raises the floor to ask for them.

## an expensive derived value, if one appears

Derived children and closure triggers recompute on every dispatch, and today every one of them is a field read or a small enum build, which the accounting section prices at approximately nothing. Suppose one of them someday does real work, say enumerating labels over an AX tree or solving a window layout. The fix for that computation is the surviving piece of pico, backdating, applied by hand: a cache owned by the node that reads the value, keyed by an `Eq` fingerprint of exactly what the computation reads.

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

A `Cached` field is state on the model, and it carries the obligations state carries: every dispatch must keep it coherent, and every test must account for it. That is why it gets adopted per computation, when a measurement shows the computation is expensive, and never as ambient infrastructure that everything routes through by default.

## changes

1. Now: the rule, binding on every future source. A source that would otherwise dispatch at sample rate reduces before it sends, in the source by default, and at the gate when the reduction reads the model. This change lands no code, because no sample-rate source exists yet. mouse-mode.md's stage-one timers do not count as one: each tick carries real work in the form of a `MoveBy`, and the timer dies with the keyup.
2. With the first sample-rate source: `RecordDetail`, the `record_detail` method with that source's variant as its first `Sample` arm, and the `dispatch_event` match, all as written above. If the source's reduction reads the model, the gate in `handle` and the mirror state land too, as written above; otherwise the source-side dedupe suffices and the model does not change.
3. If a derived computation is measured to be expensive: `Cached` in `crates/freddie`, owned by the node whose computation it caches.
