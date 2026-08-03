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

```rust
/// The generation of one entry's sync: minted by the callback, incremented per key on every
/// fact it reports. The value read queued by a fact carries the fact's generation, which is
/// what makes a stale read harmless: it names a generation the entry has moved past.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReadGen(u64);

/// A window fact: something happened to this window, and the value read for it is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowPending {
    pub window: WindowId,
    pub gen: ReadGen,
}

/// A frame read landed: what the window's frame was when the read at `gen` ran. `None` when
/// the app would not answer, which a consumer models as the frame staying unknown.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameReport {
    pub window: WindowId,
    pub gen: ReadGen,
    pub frame: Option<Frame>,
}

/// A focus fact: focus changed in the app with this pid; which window it landed on is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusPending {
    pub pid: Pid,
    pub gen: ReadGen,
}

/// A focus read landed: the focused window of `pid` when the read at `gen` ran. `None` when
/// the app has no focused window, its window has no readable id, or it would not answer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusReport {
    pub pid: Pid,
    pub gen: ReadGen,
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
    Screens(Vec<Monitor>),
}
```

`is_frontmost` is deleted. `frontmost_pid` stays for the snapshot's seed. `Pid` is `pub`.

### The callback and the worker

The callback keeps only identity resolution (`window_id(element)`, which resolves from the element in hand) and generation minting; every attribute read into the app moves to the worker, the shape `freddie_selection`'s watch specifies — one worker owning a request channel, coalescing queued requests a later generation for the same key has superseded, reporting from its own thread:

- Move/resize notification: mint the window's next `ReadGen`, report `Moved`/`Resized(WindowPending)`, queue a frame read.
- Window created: register as today, mint, report `Opened(WindowPending)`, queue a frame read.
- Focus notification and workspace activation: mint the pid's next `ReadGen`, report `FocusChanged(FocusPending)`, queue a focus read. (The activation path thereby loses its inline `focused_window_id` read; both focus sources speak one shape.)
- Destroyed / app gone: `Closed`, as today — identity only, nothing to read.

The requests:

```rust
/// One value read the worker owes. `Frame` carries the retained window element; `Focus`
/// carries only the pid, the worker creates its own app element.
enum ReadRequest {
    Frame(WindowId, ReadGen, SentElement),
    Focus(Pid, ReadGen),
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

The windows model holds the gap instead of a value:

```rust
/// One watched window's frame: known, or dead-and-in-flight since the fact at this generation.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FrameEntry {
    Pending(ReadGen),
    Known(Frame),
}
```

`WindowState.frame` becomes a `FrameEntry`. The handler arms are the selection shape: `Opened`/`Moved`/`Resized` assign `Pending(gen)`; `Frame` assigns `Known(frame)` only if the entry still holds `Pending(gen)` (a `None` frame leaves it pending); `Closed` removes, and a `Frame` report for a removed window matches nothing. A placement that finds `Pending` computes nothing — the missing value is modeled, and the response to missing is a no-op.

`focused` holds the same gap: the fact says the old answer is dead, the value read is in flight, and nothing in between shows the stale window.

```rust
/// The focused window: known, or dead-and-in-flight since the focus fact at this generation.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FocusEntry {
    Pending(ReadGen),
    /// `None` is a real answer: the front app has no focused window with a readable id.
    Known(Option<WindowId>),
}
```

Both focus rows gate on the mirror through state-produced triggers, so reports about background apps die unmatched and the handlers check nothing:

```rust
/// A focus fact about this pid. Produced from the mirrored foreground, so the row exists
/// exactly while an app is confirmed, and matches exactly its facts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusChangedFor(pub Pid);

/// A focus value about this pid. The same production, matching `Focus(FocusReport { pid, .. })`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusedFor(pub Pid);
```

```rust
    |mercury_path| mercury_path.foreground.as_ref().map(|f| FocusChangedFor(f.pid)) => if_not_invalidated(focus_pending),
    |mercury_path| mercury_path.foreground.as_ref().map(|f| FocusedFor(f.pid)) => if_not_invalidated(record_focused),
```

`focus_pending` assigns `focused = FocusEntry::Pending(gen)`; `record_focused` assigns `Known(window)` only if the entry still holds `Pending(gen)`. Whatever reads `focused` treats `Pending` as no focused window — the no-op during the gap, same as every other pending value.

Tests: the existing window tests respell to dispatch fact-then-value pairs; new tests pin the stale-frame drop (`Moved(w, n)`, `Moved(w, n+1)`, `Frame(w, n, f)` leaves `Pending(n+1)`), the same interleaving for focus, the pending placement no-op, and a `Focus` report for a background pid dying unmatched.

## Change 2: nothing

The former change 2 — bounding the synchronous callback reads — dissolves into change 1: the callback no longer reads, and the worker's reads carry the bound.

## Order of changes

One change: the protocol, the worker, the seed, and mercury's half land together — the variant shapes and their consumer cannot be split.
