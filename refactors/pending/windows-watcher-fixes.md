# the windows watcher reports facts; values follow

The windows watcher adopts the two-phase sync `freddie_selection` (`selection-watcher.md`) defines, because it has the same structure: AX notifications carry identity but not values, so today's callback fills the values in by reading the app synchronously on the main run loop, and it decides report relevance by asking the OS who is frontmost. After this doc, the callback reports only what it knows instantly — which window, which pid, what kind of change, at which generation — the value reads run on a worker, values land as second reports, a stale read names a generation the entry has moved past, and every keep-or-drop decision belongs to the consumer's model. Figaro's consumer half is `figaro/refactors/pending/sync-fixes.md` change 3, which lands after this doc's change 1.

## Change 1: the two-phase protocol

### The reports

`freddie_windows`'s change vocabulary, before:

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

after — facts and values are separate reports, and every payload is a named struct:

Generations are `freddie::Generation`, minted from one `freddie::GenerationMinter` owned by the callback — one counter for the watcher's life, never per key, so a reused pid or window id cannot alias a zombie read into a fresh entry (`synced.md`).

```rust
/// A window fact: something happened to this window, and the value read for it is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowPending {
    pub window: WindowId,
    pub generation: Generation,
}

/// A frame read landed: what the window's frame was when the read at `generation` ran. `None` when
/// the app would not answer, which a consumer models as the frame staying unknown.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameReport {
    pub window: WindowId,
    pub generation: Generation,
    pub frame: Option<Frame>,
}

/// A focus fact: focus changed in the app with this pid; which window it landed on is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusPending {
    pub pid: Pid,
    pub generation: Generation,
}

/// A focus read landed: the focused window of `pid` when the read at `generation` ran. `None` when
/// the app has no focused window, its window has no readable id, or it would not answer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusReport {
    pub pid: Pid,
    pub generation: Generation,
    pub window: Option<WindowId>,
}

pub enum WindowChange {
    /// A window appeared. Its first frame follows as a [`Frame`](Self::Frame) report.
    Opened(WindowPending),
    /// A window moved: its old frame is dead, the new one is in flight.
    Moved(WindowPending),
    /// A window was resized: same contract as [`Moved`](Self::Moved).
    Resized(WindowPending),
    Frame(FrameReport),
    /// A window went away. Final: no read is in flight for a closed window, and a `Frame`
    /// report racing the close names a window the consumer already removed.
    Closed(WindowId),
    /// Focus changed in an app — a notification's report and an activation's alike, ungated:
    /// the watcher no longer asks the OS whether the app is frontmost, it says whose focus
    /// changed and the consumer's model judges it against its own mirror.
    FocusChanged(FocusPending),
    Focus(FocusReport),
    /// The app and every entry keyed by its pid are gone. Reported by `forget_app` after the
    /// per-window `Closed` reports, so pid-keyed maps remove their entries.
    AppGone(Pid),
    Screens(Vec<Monitor>),
}
```

`is_frontmost` is deleted. `frontmost_pid` stays for the snapshot's seed. `Pid` is `pub`.

### The callback and the worker

The callback keeps only identity resolution (`window_id(element)`, which resolves from the element in hand) and generation minting; every attribute read into the app moves to the worker. One invariant, stated on `Synced` and owed here: the fact is reported through `on_change` before its read request is sent to the worker, so a value can never reach the consumer's queue ahead of its fact. Every attribute read runs on the worker, the shape `freddie_selection`'s watch specifies — one worker owning a request channel, coalescing queued requests a later generation for the same key has superseded, reporting from its own thread:

- Move/resize notification: mint the window's next `Generation`, report `Moved`/`Resized(WindowPending)`, queue a frame read.
- Window created: register as today, mint, report `Opened(WindowPending)`, queue a frame read.
- Focus notification and workspace activation: mint the pid's next `Generation`, report `FocusChanged(FocusPending)`, queue a focus read. (The activation path thereby loses its inline `focused_window_id` read; both focus sources speak one shape.)
- Destroyed / app gone: `Closed`, as today — identity only, nothing to read.

The requests:

```rust
/// One value read the worker owes. `Frame` carries the retained window element; `Focus`
/// carries only the pid, the worker creates its own app element.
enum ReadRequest {
    Frame(WindowId, Generation, SentElement),
    Focus(Pid, Generation),
}

/// A retained `AXUIElement` handed to the worker. `Send` by an explicit claim: CoreFoundation
/// objects are thread-safe to retain, release, and call, and the AX calls this crate makes off
/// the main thread are the same ones `freddie_selection`'s worker makes; the worker releases
/// it when the read is done.
struct SentElement(/* +1 AXUIElementRef, with the unsafe Send impl and its SAFETY comment */);
```

The worker performs `Frame` by reading position and size through the element and `Focus` by `AXUIElementCreateApplication` + `kAXFocusedWindow` + `window_id`. A read into a hung app blocks the worker until the OS gives up; the only consequence is entries staying `Pending` longer, which is already modeled, so nothing here tunes it.

The `elements` table stays on the main thread for placement addressing; the worker's retained elements are its own +1 references, so the two never share.

### The seed

`watch`'s install pass stops reading frames: the `Snapshot` seeds every discovered window as pending, with the reads already queued, exactly as `freddie_selection::watch` seeds its map:

```rust
/// Every window open when the watcher was installed — each pending its first frame report —
/// which one was focused, and the screens.
pub struct Snapshot {
    pub windows: Vec<WindowPending>,
    pub focused: Option<WindowId>,
    pub screens: Vec<Monitor>,
}
```

(`focused` keeps its boot read: it is the seed half of the pattern, one read at install, and `frontmost_pid` feeds it.)

### mercury's half

`ForegroundEvent` gains the pid, mirrored as `FrontApp { pid, app }`, with the mechanical respells and the constructor's added pid, exactly as figaro's `sync-fixes.md` change 2 spells it for figaro.

The model stores every report under its own key and projects at read time. Nothing is filtered on the write path: what gets stored never depends on the ordering between the windows watcher and the foreground watcher, because the join with the mirror happens when a reader asks, against state.

```rust
    /// Each watched window's frame, synced in two phases. See `freddie::Synced`.
    // WindowState.frame becomes:
    frame: Synced<Frame>,

    /// Each app's focused window, keyed by pid, synced the same way. `Known(None)` is a real
    /// answer: the app has no focused window with a readable id. Entries leave with `AppGone`.
    focused: HashMap<Pid, Synced<Option<WindowId>>>,
```

The handler arms are `Synced`'s two calls: `Opened` inserts the window's state with `frame: Synced::Pending(generation)`, `Moved`/`Resized` do `frame.change(generation)`, and `Frame` does `frame.commit(generation, f)` when the report carries one and nothing when it carries `None`; `FocusChanged` inserts `Synced::Pending(generation)` at the pid, `Focus` does `commit(generation, window)`; `Closed` removes the window, `AppGone` removes the pid's focus entry. No arm consults the foreground.

The reads are projections joining the mirror:

```rust
    /// The front app's focused window: the focus entry at `front`, if it has landed. `None`
    /// while the sync is pending, for an unreported app, and for a landed "no focused window".
    pub fn focused_window(&self, front: Pid) -> Option<WindowId> {
        *self.focused.get(&front)?.known()?
    }
```

and every placement that used `focused` calls it with the mirrored `FrontApp`'s pid; a placement that gets `None`, or finds a target frame `Pending`, computes nothing — the missing value is modeled, and the response to missing is a no-op.

Tests: the existing window tests respell to dispatch fact-then-value pairs; new tests pin the stale drops for frames and focus (`change(n)`, `change(n+1)`, `commit(n, v)` leaves `Pending(n+1)`), the pending placement no-op, `AppGone` emptying a pid's focus entry, and the ordering independence itself: a `FocusChanged`/`Focus` pair dispatched before the `Foreground` event that moves the mirror still lands in the map, and the projection answers it the moment the mirror catches up.

## Change 2: nothing

The former change 2 — bounding the synchronous callback reads — dissolves into change 1: the callback no longer reads, and how long the worker's reads take is not the model's concern.

## Order of changes

One change: the protocol, the worker, the seed, and mercury's half land together — the variant shapes and their consumer cannot be split.
