# the windows watcher reports facts; the model requests the values

The watcher stops doing two jobs. Today its callback both notices a change and fetches the new value — reading the app synchronously on the main run loop — and it decides report relevance by asking the OS who is frontmost. After this doc it only notices: every report carries what the source already holds, the consumer's model requests each missing value as a read effect, and the read's result comes back as an event that must bring the matching token half home (`freddie_sync`).

The criterion, which both repos' AGENTS.md state as doctrine: an event carries the value when the source holds it or the callback can produce it synchronously, because then no gap exists between knowing-it-changed and knowing-it; a cross-process read is asynchronous by refusal to block the callback, so its gap is modeled — `Pending` with the held half, the read effect with the riding half, `commit` as the meeting. Frames and the focused window are cross-process reads; the monitor list is a synchronous in-process read, so `Screens` keeps its value in the event.

mercury's consumer half is below; figaro's is `figaro/refactors/pending/window-sync.md`, which lands after this doc's change 1.

## Change 1: facts out, reads on demand of the model

### The reports

In `freddie_windows_types` (the vocabulary crate), before:

```rust
pub enum WindowChange {
    Opened(WindowFrame),
    Moved(WindowFrame),
    Resized(WindowFrame),
    Closed(WindowId),
    Focused(Option<WindowId>),
    Screens(Vec<Monitor>),
}
```

after:

```rust
pub enum WindowChange {
    /// A window appeared. Its frame is the consumer's read to make.
    Opened(WindowId),
    /// A window moved: its old frame is dead, and the new one is the consumer's read to make.
    Moved(WindowId),
    /// A window was resized: same contract as [`Moved`](Self::Moved).
    Resized(WindowId),
    /// A window went away. Final: a read landing after this names a window the consumer
    /// already removed.
    Closed(WindowId),
    /// Focus changed in the app with this pid — a notification's report and an activation's
    /// alike, ungated: the watcher does not ask the OS whether the app is frontmost. Which
    /// window focus landed on is the consumer's read to make.
    FocusChanged(Pid),
    /// The app and every entry keyed by its pid are gone. Reported by `forget_app` after the
    /// per-window `Closed` reports.
    AppGone(Pid),
    /// The monitors changed, with the new arrangement: reading `NSScreen` is synchronous in
    /// the callback, so no gap exists and the value rides the event.
    Screens(Vec<Monitor>),
}
```

`WindowFrame` and `Snapshot` are deleted from the vocabulary; nothing carries a frame to a consumer any more, and the seed is not a struct (below). `is_frontmost` is deleted from the watcher; `frontmost_pid` survives only to aim the install burst's `FocusChanged`.

### The callback

Identity and reporting only. `window_id(element)` resolves from the element in hand; everything else is gone: no frame reads, no `focused_window_id` on activation, no per-app gate. The move/resize/create/destroy arms and the activation observation each report their fact and return.

### The seed is the burst

`watch` returns only the `Watcher`. Its install pass reports, through `on_change` like everything else: one `Screens` with the current monitors, one `Opened(id)` per existing window, and one `FocusChanged(frontmost_pid)`. Those queue as events, dispatch after the consumer's model is constructed (watchers before seeds, as always), and the model's own minting fills its maps — the seed path and the steady-state path are the same code.

### The reads

Two functions, called by effect performers and nobody else — never by dispatch, never by the watcher:

```rust
/// The frame of `window` right now, by id through the window server. `None` for a window that
/// is gone or unreadable. Callable from any thread; an effect performer's read.
#[must_use]
pub fn frame_of(window: WindowId) -> Option<Frame>;

/// The focused window of `pid` right now, through Accessibility; the elements it creates are
/// its own and per call. `None` when the app has no focused window, its window has no readable
/// id, or it will not answer. Callable from any thread; an effect performer's read.
#[must_use]
pub fn focused_window_of(pid: Pid) -> Option<WindowId>;
```

`frame_of` reads through `CGWindowListCopyWindowInfo` filtered to the id, so it needs no element and no main thread. The watcher's element table stays exactly what it is today: the placement path's addressing, untouched by any of this.

## Change 2 (mercury): the model mints, requests, and commits

`ForegroundEvent` gains the pid, mirrored as `FrontApp { pid, app }`, with the mechanical respells and the constructor's added pid, exactly as figaro's `foreground-pid.md` spells it for figaro.

The root gains the mint, and the windows model holds the gaps:

```rust
    /// The sync tokens' mint (`freddie_sync`): a counter, so minting is a function of state.
    pub generations: GenerationMinter,
```

```rust
    // WindowState.frame becomes:
    frame: Synced<Frame>,

    /// Each app's focused window, keyed by pid. `Known(None)` is a real answer: the app has
    /// no focused window with a readable id. Entries leave with `AppGone`.
    focused: HashMap<Pid, Synced<Option<WindowId>>>,
```

Two effects and two value events:

```rust
    /// Read `window`'s frame off the effect loop; the answer returns as [`FrameRead`] carrying
    /// this half.
    ReadFrame { window: WindowId, generation: RidingGeneration },
    /// Read `pid`'s focused window; the answer returns as [`FocusRead`] carrying this half.
    ReadFocus { pid: Pid, generation: RidingGeneration },
```

```rust
pub struct FrameRead {
    pub window: WindowId,
    pub generation: RidingGeneration,
    /// `None` when the read could not answer; the entry stays `Pending`.
    pub frame: Option<Frame>,
}

pub struct FocusRead {
    pub pid: Pid,
    pub generation: RidingGeneration,
    pub window: Option<WindowId>,
}
```

The handler arms, all on the root's windows handler:

- `Opened(id)`: mint; insert the window's state with `frame: Synced::Pending(held)`; emit `ReadFrame { window: id, generation: riding }`.
- `Moved(id)` / `Resized(id)`: mint; `frame.change(held)`; emit the same `ReadFrame`.
- `FrameRead`: `frame.commit(&ev.generation, frame)` when the report carries one; nothing when it carries `None`.
- `FocusChanged(pid)`: mint; insert `Synced::Pending(held)` at the pid; emit `ReadFocus`.
- `FocusRead`: the pid's entry `commit(&ev.generation, ev.window)`.
- `Closed(id)`: remove the window; `AppGone(pid)`: remove the pid's focus entry; `Screens(monitors)`: assign, as today.

No arm consults the foreground; nothing is filtered on the write path. The performers are dumb threads: `ReadFrame` runs `freddie_windows::frame_of` and sends `FrameRead`; `ReadFocus` runs `focused_window_of` and sends `FocusRead`; each moves its riding half through and into the event.

The reads are projections joining the mirror, unchanged from the previous revision:

```rust
    /// The front app's focused window: the focus entry at `front`, if it has landed. `None`
    /// while the sync is pending, for an unreported app, and for a landed "no focused window".
    pub fn focused_window(&self, front: Pid) -> Option<WindowId> {
        *self.focused.get(&front)?.known()?
    }
```

and a placement that gets `None`, or finds a target frame `Pending`, computes nothing.

Tests: each fact arm asserts its `Pending` insert and its emitted read effect; each value arm asserts the commit; the stale interleaving (`Moved`, `Moved`, first `FrameRead`) asserts the first riding half no longer matches; a `FocusRead` for a background pid still lands in the map and the projection answers only when the mirror points at it; the seed burst dispatched in order leaves a fully pending model that converges as the reads return.

## Order of changes

Two: change 1 (the vocabulary, the callback, the burst, the read functions — with mercury's compile fixed in the same commit, since the variant shapes change under it) and change 2 (mercury's minting model). Figaro's half rides `sync-fixes.md` and lands after change 1.
