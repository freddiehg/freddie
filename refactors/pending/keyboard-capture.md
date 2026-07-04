# capturing keyboard events

Three primitives: capture, emit, drop. A `Capture` grabs the keyboard, delivers the keys it intercepts, and emits keys back; dropping it releases. Multiple captures stack.

## The API

```rust
// freddie_keyboard

pub use freddie_keys::{Key, KeyEvent};

pub struct Capture(/* private */);
impl Capture {
    // Grab every key; `on_key` runs per intercepted key. Drop the Capture to release.
    pub fn grab(on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<Capture, CaptureError>;
    pub fn builder() -> Builder;                       // capture only chosen keys

    pub fn tap(&self, key: Key) -> Result<(), EmitError>;      // press then release
    pub fn press(&self, key: Key) -> Result<Press, EmitError>; // held; released when the last handle drops
}

pub struct Builder(/* private */);
impl Builder {
    pub fn key(self, key: Key) -> Builder;
    pub fn grab(self, on_key: impl Fn(KeyEvent) + Send + 'static) -> Result<Capture, CaptureError>;
}

pub struct Press(/* private */);
impl Clone for Press;   // another handle keeping the key down
// impl Drop for Press: the last handle releases the key.

pub struct CaptureError;                 // Display + Error
pub enum EmitError {
    Unmappable(Key),   // no key code on this OS (a named F21-F24; Raw always has a code)
    Post,              // the OS refused to build or post the event
}
```

You provide `on_key` when you grab, so keys are never buffered inside freddie_keyboard; each is handled the moment it arrives. `on_key` runs on the capture thread, so keep it to a forward and do the work elsewhere: mercury sends the key into a tokio channel and dispatches in its event loop, which is how the whole thing stays tokio-friendly without freddie_keyboard owning a runtime. `on_key` is `Send + 'static`; emit (`tap`/`press`) runs on your own thread through the `Capture`, not inside `on_key`. `Capture` and `Press` use `Rc` and are `!Send`; a `send` Cargo feature swaps `Rc` for `Arc` (README TODO).

A pull form is also exposed, and it is a footgun: a grab whose keys you drain yourself. Convenient, but it buffers unboundedly if you fall behind, so `on_key` is the default and the pull form is opt-in.

`Key` is one enum, every key a variant (`Key::KeyR`, `Key::Escape`, `Key::MetaLeft`, `Key::Raw(u16)`), so keys match, hash, and pass uniformly. Separate per-key structs would block all of that.

`KeyEvent` is `{ key: Key, down: bool }`: which key, and whether it went down or up. Nothing else. Keyboards are not velocity-sensitive (that is MIDI), and the auto-repeat flag and timestamp the OS also exposes are not relevant to v1.

## tap, press, and holding

`tap(key)` presses and releases. `press(key)` presses and returns a `Press` that releases the key when it drops. Held keys are ref-counted per key: pressing a key already held (a second `press`, or `Press::clone`) does not re-press it, and it stays down until the last handle drops. A crash or early return drops the handles and frees everything held.

The per-key count and the event source live in the `Capture`. A `Press` holds a reference into that state (an `Rc` the `Capture` also holds) plus its key; `Press::clone` adds a holder, `Press::drop` removes one and emits the release at zero.

There are no chords. cmd+r is composed:

```rust
let _cmd = capture.press(Key::MetaLeft)?;   // cmd down, held
capture.tap(Key::KeyR)?;                      // r down, r up
// _cmd drops at scope end: cmd up
```

```rust
// Rc<HeldState> lives in Capture; each Press clones it. Mutex/Arc under the `send` feature.
struct HeldState { counts: RefCell<HashMap<Key, usize>>, source: CGEventSource }
pub struct Press { state: Rc<HeldState>, key: Key }

impl Capture {
    pub fn press(&self, key: Key) -> Result<Press, EmitError> {
        let code = to_code(key).ok_or(EmitError::Unmappable(key))?;
        let mut counts = self.state.counts.borrow_mut();
        let n = counts.entry(key).or_insert(0);
        if *n == 0 { post(&self.state.source, code, /* down */ true)?; }   // 0 -> 1: key-down, tagged
        *n += 1;
        Ok(Press { state: self.state.clone(), key })
    }
}
impl Drop for Press {
    fn drop(&mut self) {
        let n = self.state.counts.borrow_mut().entry(self.key).or_insert(0);
        *n -= 1;
        if *n == 0 { let _ = to_code(self.key).map(|c| post(&self.state.source, c, false)); }
    }
}
```

## EmitError

- `Unmappable(key)`: the key has no key code on this OS. Named keys like F21-F24 on macOS. `Key::Raw(code)` always has a code, so it is never `Unmappable`.
- `Post`: the OS refused to build or post the event.

The source is created once at `grab`, so a source failure is a `CaptureError` there, not an `EmitError`.

## Pipeline

Captures form a pipeline, not a fan-out. A physical key enters the first stage; that stage's `on_key` runs and emits a possibly changed key; the next stage captures that emit and emits again; the last stage's output reaches the app. Each `grab` (or `builder().grab`) adds a stage.

- Only the first stage sees the raw key; every later stage sees the output of the stage before it.
- A stage ignores its own emit and handles everything else, so it never re-handles what it just produced but does handle the stage before it.
- A `builder` stage only captures its listed keys; keys it does not list flow past to the next stage untouched.
- New stages attach at the end, downstream and nearest the app, so existing stages transform first and the new stage sees the already-modified keys. You cannot insert into the middle.
- Dropping a stage removes it and the pipeline closes the gap. You still cannot insert into the middle afterward, only at the end.

Two stages that touch different keys leave each other's keys alone. A stage that maps caps-lock to escape, followed by one that maps escape onward, chains: caps-lock becomes escape becomes whatever the second stage makes it.

The in-process pipeline and the cross-process one are separate, independent layers.

Within a process the pipeline is just a `Vec` of stages: one OS tap feeds the first, the event threads through the vec in software, and the last stage's output is posted once. Trivial, generic, and unit-testable with no tap. This is all the same-process requirement needs.

The cross-process version is the same pipeline semantics for when the stages cannot live in one vec because they are in different processes. Instead of one owning vec, the per-process pipelines are linked through the global OS event-tap chain, which stands in for the vec's ordering, so one process's posted output is the next's tap input. The vec pipeline knows nothing about this; cross-process is added on top. Same-process is the requirement, cross-process the goal, and the two are separable.

## Generic over the event

Nothing above mentions keys, so the pipeline is `Capture<T>` where `T: Intercept`. `Intercept` is everything the backend needs from an event type:

```rust
pub trait Intercept: Sized + Send + 'static {
    fn tap_types() -> &'static [CGEventType];                  // which events to tap
    fn decode(raw: &CGEvent) -> Option<Self>;                 // OS event -> T
    fn encode(&self, src: &CGEventSource) -> Option<CGEvent>; // T -> OS event, for emit
}
```

`Capture<T>`, `grab(on_key: impl Fn(T))`, `emit(&self, T)`, stacking, and drop are all generic; only the `Intercept` impl is per-event. `KeyEvent` impls `Intercept` (`decode`/`encode` through the keycode table, tap `[KeyDown, KeyUp, FlagsChanged]`). A mouse event type would impl it with mouse `CGEventType`s and fields, reusing the whole pipeline.

freddie_keyboard is then `Capture<KeyEvent>` plus the keyboard sugar: `tap(key)` and `press(key)` compose `emit(KeyEvent { key, down })`, and `Press` is the held-key refcount. Those sit on top of the generic `emit`. (`CGEvent` in the trait is the macOS raw type; going cross-platform makes that raw type a backend parameter.)

## Unit tests

The pipeline threading is a pure function over the ordered stages, unit-tested with no tap:

- Two stages, A->B then B->C: a raw A comes out the far end as C.
- A stage leaves keys it does not touch alone, so they reach the next stage unchanged.
- Three stages, drop the middle: the pipeline reconnects, first straight to last.
- A new stage lands at the end, after every existing stage, never in the middle.

## Loop prevention

Loop prevention is per process, not per stage. The stages thread in memory, so handing an event to the next stage never round-trips the OS. Only the pipeline's final output is posted, stamped in `USER_DATA` with the process's tag; that process's tap passes its own tag straight through so it does not re-capture its own output, while another process's tap (a different tag) does. Without it a process would post a key, its own tap would see it, and it would loop.

## The macOS backend

In-process there is one `CGEventTap`. Its callback ignores the process's own output (the tag), decodes the event, and feeds it into the vec pipeline; the pipeline posts outputs back, tagged.

```rust
|_, kind, event| {
    if event.field(USER_DATA) == MY_TAG { return Keep; }   // our own output: let it out
    match KeyEvent::decode(event) {                        // T::decode
        Some(ev) => { pipeline.feed(ev); Drop }            // into the in-memory vec of stages
        None     => Keep,
    }
}
```

`with_enabled` does the run-loop wiring inside core-graphics, so we call no `unsafe`. `CFRunLoop` is `Send`, so dropping the pipeline stops the loop from any thread. A process exit or `kill -9` releases the tap, since the WindowServer drops taps for dead processes. The cross-process layer adds nothing to this callback; it relies on the OS chain to order another process's tap around ours, with the per-process tag keeping each process from eating its own output.

## Async emit

`on_key` is async: the stage forwards the key and emits later, off the tap callback. That emit injects the key into the pipeline just downstream of this stage, so it reaches the next stage and never the stage itself or the ones before it:

```
Capture1 gets A, forwards it, decides to swallow A and emit "a"
Capture2 gets nothing yet
... later ...
Capture1 emits "a"   ->  injected just after Capture1
Capture2 gets "a"
```

Because the emit lands downstream of its own stage, no stage sees its own output, so the per-stage tag is a backstop, not the primary guard. Position does the work.

The pipeline model does not need to know how that injection happens; it is a backend concern. In-process, the backend threads an emit to the next stage itself, so it controls the position exactly and this is simple. Cross-process leans on the global OS tap chain and is the harder, later case. How a `press` hold spans events is the other backend detail.

## Left, right, and modifiers

`Key` splits the sides already: `ShiftLeft`/`ShiftRight`, `ControlLeft`/`ControlRight`, `AltLeft`/`AltRight`, `MetaLeft`/`MetaRight`, each with its own macOS keycode, so capture reports the exact key. But modifiers do not arrive as `KeyDown`/`KeyUp`; macOS sends `FlagsChanged`. So the tap listens for `[KeyDown, KeyUp, FlagsChanged]`, and for a `FlagsChanged` it reads the keycode (which carries the side) and infers press or release from whether that modifier's flag bit just turned on or off.

## Custom and unmapped keys

macOS keycodes are arbitrary `u16`: `CGEventCreateKeyboardEvent` takes any code and the tap reads any code, so made-up keys can be captured and emitted. A code produces a character only if the active layout maps it, so a custom code is a remap intermediary (emit it, catch it in your own tap), not typed text. `Key::Raw(u16)` carries such a code, and `from_code` returns `Raw(code)` for anything the named table lacks, so no key is dropped for being unnamed. The `u16` is the native code, which makes `Raw` the one part not portable across OSes. Linux and Windows are scancode/keycode based too, so likely the same, but that is unconfirmed.

## Escape and quitting are not here

`freddie_keyboard` has no notion of escape or quit. That is the consumer's, jerry-rigged until the hijack is trusted:

```rust
let _capture = Capture::grab(move |ev| {
    if ev.key == Key::Escape { process::exit(0); }   // mercury's, not the keyboard's
    if ev.down { let _ = event_tx.send(ev); }         // forward into mercury's tokio channel
})?;
```

## Ordering caveat

A stage passes a key it does not capture straight to the next stage (synchronous `Keep`), while a key it captures is swallowed and re-emitted later (async). So a passed key can overtake a captured one still in flight (passthrough.md). Fine for command keys; watch it for inline remaps. mercury's one stage captures everything, so nothing is passed synchronously and there is no reorder.

## What's left to build

The code today has `run(on_key)` + `emit(key)` + `emit_chord(mods, key)` on the core-graphics backend, with mercury driving it. To reach the API above:

1. `Capture` as a pipeline stage: `grab(on_key)` / `builder().grab(on_key)` install a tap appended to the chain, `on_key` runs per key, `Drop` removes the stage.
2. Emit on `Capture`: `tap` and `press -> Press` backed by a per-key count and a source held in the `Capture` (`Rc`, `Arc` under `send`); an async emit injects downstream of the stage.
3. `Key::Raw(u16)` in `freddie_keys`, and `FlagsChanged` in the tap for modifiers.
4. mercury: `on_key` forwards each key into a tokio channel (escape exits from `on_key`); the event loop dispatches and re-emits through the `Capture`; one capture-all stage, so `tap` on a re-emit.
5. The pull (footgun) grab variant.

## Known v1 limits

- A key with no named `Key` mapping is delivered as `Raw(code)`, so nothing is lost, but that code is the non-portable macOS value.
- If macOS disables the tap on a timeout it does not auto re-enable (the cost of `with_enabled`, which keeps us unsafe-free). The callback is trivial, so it should not fire.
- F21-F24 have no macOS keycode, so emitting them returns `Unmappable` until we learn what real hardware sends.

## Permissions

An active tap needs Accessibility plus Input Monitoring, granted to whatever launches the binary. First run prompts; flip it in System Settings and relaunch.
