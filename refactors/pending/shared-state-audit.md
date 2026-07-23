# bringing the shared-state sites in line with the standard

`CLAUDE.md`'s "Shared state and interior mutability" section says `Arc`, `Rc`, `Mutex`, `RwLock`, `Cell`, `RefCell`, `OnceLock`, `lazy_static`, `thread_local!`, and atomics are almost always the wrong reach: the model is a single-threaded pure function of state and event, and the preferred way to move data between threads is a channel whose sender is freely `Send` while the receiver stays pinned to one thread.

Seven files predate that rule. This doc changes the ones that fail the standard and records the verdict on the ones the constraint genuinely forces, so a later audit does not re-open them. `freddie_overlay` is its own change and moved to `refactors/pending/overlay-marshaling.md`.

Full survey of the tree: the ten other crates (`bind`, `bind_macro`, `derive_support`, `freddie_keys`, `freddie_keyboard`, `freddie_menu_bar`, `freddie_single_instance`, `freddie_app_nav`, `laserbeam`, `mercury`) use none of these primitives.

## Change 1: `freddie_main_loop`'s stop signal becomes a channel

Landed in `0f97992`. The `Arc<AtomicBool>` is a `std::sync::mpsc` channel now: `Stopper` holds the `Sender` and sends before `CFRunLoop::stop` in its `Drop`, `MainLoop` holds the `Receiver` and keeps turning while `try_recv` is `Empty`. The `main_loop() -> (MainLoop, Stopper)` signature is unchanged, so `daemon.rs` was untouched.

## Change 2: `freddie/src/timer.rs`'s id source moves onto the root

`crates/freddie/src/timer.rs:29`:

```rust
fn mint() -> Self {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    Self(NEXT.fetch_add(1, Ordering::Relaxed))
}
```

The comment already admits the atomic synchronizes nothing: "Atomic because a mutable static has to be `Sync`, not because anything sets timers off one thread." It is ambient, process-global, and it makes every dispatch that arms a timer impure. Under `CLAUDE.md`'s ambient-state rule the counter belongs on the root model, minted through a newtype so the id is a function of state.

The id becoming deterministic pays for itself in the model tests: `TimerFired`'s `testing`-only "always equal" impl exists only because a test could not predict the id a global atomic handed out. With the counter on root, the id in a produced effect is the root's counter before the increment, which a test that builds the expected state knows, so the id becomes assertable and the hack goes.

This change also retires the per-call closure. `timer_effect_and_guard` builds the event through `From<TimerFired>` instead of a `|id| …` closure, so the arming sites name a delay and the id source. The `MercuryEffect::Timer(effect)` wrap stays where it already is, at the use sites.

### The id source newtype

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
/// of one armed later, or a stale event would match a fresh guard. The read-then-bump lives here and
/// nowhere else; a caller only ever holds `&mut TimerIds` and never sees an id.
#[derive(Default, Debug)]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
pub struct TimerIds(u64);

impl TimerIds {
    /// The next id, advancing the source. The only way to make a [`TimerId`], so no caller mints
    /// off an ambient counter.
    fn next(&mut self) -> TimerId {
        let id = TimerId(self.0);
        self.0 += 1;
        id
    }
}
```

### The generic `timer_effect_and_guard`

The closure `impl FnOnce(TimerId) -> E` existed so the caller could bake the minted id into an event. Every caller built the same thing, `Event::Timer(TimerFired(id))`, so a `From<TimerFired>` bound on the event type expresses it once and the closure goes.

`timer_effect_and_guard`, before:

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
    ids: &mut TimerIds,
    delay: Duration,
) -> (TimerGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    let id = ids.next();
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

The return type is unchanged: it still hands back `TimerEffect<E>`, and the use site wraps it into `MercuryEffect::Timer(...)` as it already did. `E` is inferred from that wrap, which requires `TimerEffect<MercuryEvent>`, so no call needs a turbofish. No custom trait is introduced: a one-implementor trait that returned the consumer's effect directly would only fold a wrap that is not duplicated, against `CLAUDE.md`'s rule on custom traits.

`crates/mercury/src/model.rs`, added beside `MercuryEvent`:

```rust
impl From<TimerFired> for MercuryEvent {
    fn from(fired: TimerFired) -> Self {
        MercuryEvent::Timer(fired)
    }
}
```

`crates/freddie/src/lib.rs` re-exports `TimerIds` alongside the existing timer exports.

### `TimerFired` derives its comparison again

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

`TimerEffect` keeps its `#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]` and its `AlwaysEqual<oneshot::Receiver<()>>` on `cancel`: the receiver is incomparable whatever the id does, so that wrapper is unrelated to this change and stays. Its `event` field now compares by the real id, since `TimerFired` does. This does not give `MercuryEffect` or `MercuryEvent` an `Eq`: a window frame is four `f64`s, so they stay `PartialEq`-only, exactly as before.

### The counter on the root

`crates/mercury/src/state/mod.rs`, the `Mercury` struct gains a field:

```rust
    /// The source of timer ids. On the root because the program mints these; every arm draws the
    /// next from here, so an id is a function of state. See `CLAUDE.md`'s ambient-state rule.
    timer_ids: TimerIds,
```

`TimerIds` derives `Default`, so `Mercury`'s construction is unchanged where it is `..Default::default()`; a hand-written constructor sets `timer_ids: TimerIds::default()`.

### The arming sites lose their closure

Root methods reach the field directly. `toggle_overlay` (`mod.rs:619`), before:

```rust
        let (guard, effect) =
            timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
        self.overlay = Some(guard);
```

after:

```rust
        // `&mut self.timer_ids` is borrowed only for this call; `guard` is owned (a `Copy` id and a
        // `DropGuard`, no reference to the source), so the borrow is gone before the next line stores
        // it into the `overlay` field. Bump and store are two sequential statements, never one borrow.
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
    fn asking_for(&mut self, ids: &mut TimerIds, target: WindowFrame) -> Vec<MercuryEffect> {
        let (timer, effect) = timer_effect_and_guard(ids, PLACEMENT_SETTLE);
```

### `arm_return_home` stays; `arm_jk_timeout` goes

Once the closure and the wrap are in the framework, the two helpers add only a delay. `arm_jk_timeout` has one caller and passes its delay straight through, so it is inlined. `arm_return_home` has six callers and is the one place that names the return-home timer's delay, so it stays as a one-liner.

`crates/mercury/src/state/mod.rs`, before:

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
fn arm_return_home(ids: &mut TimerIds) -> (TimerGuard, MercuryEffect) {
    let (guard, effect) = timer_effect_and_guard(ids, RETURN_TO_HOME_TIMEOUT);
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

`root` (`node.parent`) already exposes `Mercury`'s fields mutably here, so `&mut root.timer_ids` needs no ascend. `root.timer_ids` and `root.typing_state` are disjoint fields, so the borrows do not collide.

### Threading the source through the layer transitions

Minting borrows `root.timer_ids` only for the `timer_effect_and_guard` call: the `TimerGuard` it returns is owned and borrows nothing, so the borrow ends at the call and the guard is then free to be stored. Storing it is one of two shapes, and neither collides with the mint. Either the guard goes into a value being built (a layer constructor owns it and `set_layer` installs it), or it goes onto an existing field (`toggle_overlay`'s `self.overlay`, the jk handler's `root.typing_state.jk`), which is a different field of `Mercury` than `timer_ids`. Where the target is a sub-struct method, the caller passes both as disjoint fields, `root.windows.placing(&mut root.timer_ids, target)`, which the borrow checker accepts. Ascending to the root is what supplies the `&mut` for both halves; it does not lock anything.

The layer constructors that arm a return-home timer take the source. `NavLayer::new` (`state/nav.rs:39`) is representative; `AppLayer::new` (`app.rs`), `SiteLayer::new` (`site.rs`), and `ResizeLayer::new` (`resize.rs`) are the same shape. before:

```rust
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home();
```

after:

```rust
    pub(crate) fn new(ids: &mut TimerIds) -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home(ids);
```

The transition handlers call a constructor then `set_layer` on the ascended root, so they need `&mut` to the root to hand over `&mut root.timer_ids`. `ascend_mut` supplies it: `refactors/pending/ascend-by-ref.md` (which lands first) renames the consuming walk to `ascend_mut(self) -> Target`, and for the root `Target` is `MercuryPath<'a>`, an `&mut Mercury`. `handlers/home.rs`'s nav transition is representative, before (already `ascend_mut` once `ascend-by-ref` has renamed it):

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

`root` is an `&mut Mercury`, and `root.timer_ids` and the layer field `set_layer` writes are disjoint fields, so the borrow for `new` is released before `set_layer` reborrows `*root`.

Every call to a `*Layer::new()` is updated to pass the source: the `home.rs` transitions (`home.rs:35,57,71,82`), and the `handlers/nav.rs` in-app transition (`nav.rs:24`). `Windows::placing`'s call to `asking_for` forwards its own `&mut TimerIds`.

### The tests

`crates/mercury/tests/transitions.rs` builds expected timer effects and now needs a source to mint from. The helpers at `transitions.rs:26,33,1209,1448` gain a `&mut TimerIds` argument threaded from the test's expected state and drop the `fired` closure, wrapping the `TimerEffect` with `MercuryEffect::Timer` as the code under test does, so the id in the expected effect matches the id the transition minted. The `fired` free function (`transitions.rs:39`) goes with the closure.

`timer_id(effects)` (`transitions.rs:44`), which reads the id off a produced effect, is unaffected: it reads whatever the transition minted, and that is now a function of the state the test dispatched against.

## Change 3: `freddie_overlay`'s marshaling to the main thread

Moved to `refactors/pending/overlay-marshaling.md`. It replaces the sink's whole delivery path (GCD dispatch into a `thread_local` table becomes a channel drained on `MainLoop::run`'s `on_wake`), which is a design pass of its own rather than a mechanical edit.

## Change 4: `freddie_event_socket`'s handler funnels through one task

`crates/freddie_event_socket/src/lib.rs:63`:

```rust
let on_message = Arc::new(on_message);
// ...
tokio::spawn(serve(stream, Arc::clone(&on_message), closed.clone()));
```

`Arc<F>` shares one immutable `Fn` across the connection tasks. Replace it with the preferred shape: one task owns `on_message`, and each connection forwards its frames over an `mpsc` whose `Sender` is `Send` and `Clone`. `on_message` is already required non-blocking, so serializing the calls onto one task costs nothing, and it drops both the `Arc` and the `Sync` bound the shared `Fn` needed.

`crates/freddie_event_socket/src/lib.rs`, `use tokio::sync::mpsc;` is added beside the `watch` import.

`listen`, before:

```rust
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    let on_message = Arc::new(on_message);

    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = dropped(&mut closed) => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted");
                    tokio::spawn(serve(stream, Arc::clone(&on_message), closed.clone()));
                }
                // A refused connection is that client's problem; the listener keeps accepting.
                Err(e) => debug!(error = %e, "accept failed"),
            }
        }
        debug!("the event socket closed");
    });

    Ok(EventSocket {
        _shutdown: shutdown,
    })
}
```

after:

```rust
pub fn listen<F>(port: u16, on_message: F) -> io::Result<EventSocket>
where
    F: Fn(&str) + Send + 'static,
{
    let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    let (shutdown, mut closed) = watch::channel(());
    // Every connection forwards its frames here; one task owns `on_message` and drains them. The
    // sender is `Send` and `Clone`, the receiver stays on the one task, which is the shape the
    // shared `Arc<F>` was standing in for. Unbounded because the drain is the non-blocking
    // `on_message` and keeps up; a frame is capped at `MAX_FRAME_BYTES` either way.
    let (frames, mut incoming) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        while let Some(frame) = incoming.recv().await {
            on_message(&frame);
        }
        debug!("the event socket's dispatch ended");
    });

    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = dropped(&mut closed) => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted");
                    tokio::spawn(serve(stream, frames.clone(), closed.clone()));
                }
                // A refused connection is that client's problem; the listener keeps accepting.
                Err(e) => debug!(error = %e, "accept failed"),
            }
        }
        debug!("the event socket closed");
    });

    Ok(EventSocket {
        _shutdown: shutdown,
    })
}
```

The dispatch task ends on its own: when the socket drops, the accept loop breaks and drops the original `frames`, each `serve` ends and drops its clone, and `incoming.recv()` then returns `None`.

`serve` stops being generic and forwards to the channel. before:

```rust
async fn serve<F>(stream: TcpStream, on_message: Arc<F>, mut closed: watch::Receiver<()>)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    // ...
        match frame {
            Some(Ok(Message::Text(text))) => on_message(text.as_str()),
            Some(Ok(Message::Binary(_))) => debug!("dropping a binary frame"),
```

after:

```rust
async fn serve(stream: TcpStream, frames: mpsc::UnboundedSender<String>, mut closed: watch::Receiver<()>) {
    // ...
        match frame {
            // The receiver is dropped only after the socket is gone, which the `dropped` arm above
            // catches first; a send that still loses that race is a frame arriving as we close, and
            // dropping it is what closing means.
            Some(Ok(Message::Text(text))) => {
                let _ = frames.send(text.as_str().to_owned());
            }
            Some(Ok(Message::Binary(_))) => debug!("dropping a binary frame"),
```

`use std::sync::Arc;` is removed. The mercury caller's closure (`daemon.rs`, which sends on the event channel) already satisfies `Fn(&str) + Send + 'static`, so it is untouched. The `tests/socket.rs` recorder is unaffected: its `on_message` no longer needs `Sync`, which it had.

## Reviewed and kept

These sites hold a primitive because a platform or trait constraint forces it. Each gets one line added to the type or function's doc comment recording that the constraint was reviewed against the standard and stands, so the next audit does not re-litigate.

- `crates/freddie_windows/src/lib.rs:142` `Elements(Mutex<HashMap<WindowId, Arc<Element>>>)`, the one genuinely cross-thread case. The strong `Arc<Elements>` is on `WatcherState` (`:556`), held by the main thread; `WindowSink` (`:153`) holds a `Weak<Elements>` and upgrades it to `Arc` per call (`:985`), so the `Mutex` is the thing that actually crosses threads. `WindowSink::set_frame` runs off the main thread by design (tens of milliseconds per placement, which the run loop cannot spend) while the main thread mutates the table as windows open and close. A channel to the main thread would put those milliseconds back on the run loop, which is the thing the design avoids. The `Weak` is deliberate: the watcher holds the only strong reference, so dropping the watcher ends every sink's access.
- `crates/freddie_windows/src/lib.rs:976,559` `state: Rc<WatcherState>` and `apps: RefCell<HashMap<Pid, AppObserver>>`. The C-callback-reaches-state pattern from `docs/platform-apis.md`: `AXObserver` callbacks reach state through a `refcon`, and the `'static` launch and terminate closures cannot borrow the `Watcher`. Single-threaded shared ownership with C callbacks.
- `crates/freddie_cli/src/logging.rs:86` `thread_local! { static STAMPED: RefCell<Vec<u8>> }`. `io::Write::write` is a synchronous trait method `tracing` calls on whatever thread logs, so there is no owning thread to send to. The buffer is per-thread scratch reused to avoid a per-record allocation on the daemon's hot path.
- `crates/freddie_cli/src/logging.rs:94` `static STAMP: OnceLock<String>`. The process pid string, built once and immutable. This one could instead be computed at `init` and owned inside `WithPid<W>`, removing the `OnceLock`; fold that in when `logging.rs` is next touched, but it is not worth a standalone change.
- `crates/freddie_event_socket/tests/socket.rs:19` `Arc<Mutex<Vec<String>>>`. A test recorder. `CLAUDE.md` scopes the model standard to the model, and a test that collects what a callback saw is outside it.
