# Seeding at construction

A source that has a current value at boot supplies it to `Mercury`'s constructor. Nothing is seeded by sending an event.

Today the frontmost app arrives twice, from two separate calls to `freddie_app_nav::frontmost()`: once as a `Foreground` event queued before the event loop starts, and once as a field written onto the model after it is built. The dispatch changes nothing, which is what a run's second log line says:

```
initial state state=Mercury { foreground: Foreground { app: Ghostty, .. }, .. }
dispatch event=Foreground(ForegroundEvent { app: Ghostty }) effects=[] state=Mercury { foreground: Foreground { app: Ghostty, .. }, .. }
```

The event is the half to remove. A model seeded by event is built claiming `App::Other` and corrected a tick later, so for that tick it asserts something false — the in-app layer resolves against an app that is not frontmost, and a key arriving in the gap is dispatched against it. Construction is where a starting value belongs, because a `Mercury` that has never been told what is frontmost should not be a value anyone can hold.

An event still means what it has always meant: something changed. A seed is not a change.

The boundary is `main_loop.run`. Before it, the process is constructing itself, and reading the OS is how it finds out what to construct: `freddie_app_nav::frontmost`, the window snapshot, the screen list. Once the loop is running, no read is allowed and every fact arrives as an event. That is the rule the seeds have to satisfy, and it is why they are read on the main thread and handed to the worker as data rather than read by the worker while the loop is already turning.

A source whose value is genuinely unknown at boot is unaffected and stays `None`. The Chrome tab URL is the example: no extension has connected, so there is nothing to seed, and `None` is the honest answer rather than a placeholder waiting to be corrected.

---

# Change 1: `Mercury::new` takes what is known at boot

`Mercury::default()` followed by a write is itself construction in two steps, and it leaves the half-built value reachable. A constructor closes that too.

`crates/mercury/src/state/mod.rs`:

```rust
impl Mercury {
    /// The model at boot, told what the sources already know.
    ///
    /// `front_app` comes from `freddie_app_nav::frontmost`. There is no `Default`: a
    /// `Mercury` that has not been told what is frontmost would answer `App::Other`, and
    /// the in-app layer would resolve against the wrong app until something corrected it.
    #[must_use]
    pub fn new(front_app: App) -> Self {
        let mut mercury = Self {
            foreground: Foreground::default(),
            typing_state: TypingState::default(),
            overlay: None,
            layer: Layer::default(),
        };
        mercury.foreground.set_front_app(front_app);
        mercury
    }
}
```

`#[derive(Default)]` comes off `Mercury`. The tests build one through `Mercury::new(App::Other)`, which says what they mean: no particular app is frontmost.

# Change 2: the seed event goes

`crates/mercury/src/daemon.rs`, in `serve`.

Removed:

```rust
    // The app-navigation source. `watch` reports changes, not the app that is
    // already frontmost, so seed that one by hand.
    if let Some(bundle_id) = freddie_app_nav::frontmost() {
        let _ = event_tx.send(foreground(App::from_bundle_id(&bundle_id)));
    }
```

The comment above `watch` keeps the half that is still true:

```rust
    // `watch` reports changes, not the app that is already frontmost; that one went to
    // `Mercury::new`. The block runs on the main thread, where callbacks are serialized,
    // so it does one send and returns. The work happens back on this thread.
    let _watcher = freddie_app_nav::watch({
```

Before:

```rust
    // Seed the model with the app that is actually frontmost, rather than defaulting to
    // `Other`, so the in-app layer resolves correctly before the first foreground event.
    let mut mercury = Mercury::default();
    mercury.foreground.set_front_app(
        freddie_app_nav::frontmost()
            .map_or(App::Other, |bundle_id| App::from_bundle_id(&bundle_id)),
    );
```

After, taking what `main` read and passed in:

```rust
    let mercury = Mercury::new(front_app);
```

One call to `frontmost`, one place the model learns what is frontmost, and no dispatch that changes nothing.

# Change 3: the seed is read on main, before the loop runs

`frontmost()` is called on the worker today. The worker is spawned before `main_loop.run`, but it runs alongside it, so the read is not ordered before the loop starts — it only usually happens first.

Everything read before the loop goes into one value, so the boundary is a type and not a rule someone remembers. It lives in mercury: `freddie_main_loop` is generic and must not learn what an `App` is.

`crates/mercury/src/daemon.rs`:

```rust
/// What the process read from the OS before the main loop started turning.
///
/// Reading the OS is allowed while this is being built and at no point after. Once
/// `main_loop.run` is going, every fact reaches the model as an event, so anything the
/// model needs to start from has to be in here.
struct Boot {
    /// The app that was already frontmost. `freddie_app_nav::watch` reports changes, and
    /// at boot nothing has changed yet.
    front_app: App,
}
```

Built in `main`, beside the event channel it already builds there:

```rust
    let boot = Boot {
        front_app: freddie_app_nav::frontmost()
            .map_or(App::Other, |bundle_id| App::from_bundle_id(&bundle_id)),
    };
```

`serve` takes `boot` alongside the channels it already takes, and hands `boot.front_app` to `Mercury::new`. `freddie_app_nav::watch` stays on the worker: registering is thread-agnostic, and delivery is on main either way.

`refactors/pending/window-observation.md` adds the window snapshot and the `WindowSink` to this struct rather than to `serve`'s parameter list, which is what keeps a growing set of sources from growing an argument list.

# Change 4: the boot log stops being a separate kind of line

`initial state` exists because the model was finished in two places and the log had to show the first one. With one, it is the same fact the first real dispatch already carries.

Before:

```rust
    info!(?state, "initial state");
```

After: unchanged in content, but it now describes a model that is complete rather than one awaiting correction. It stays `info!`, since the state a run begins in is worth having in the log without raising the level.

The line that goes is the redundant `dispatch event=Foreground(..) effects=[]`, and it goes by not existing rather than by being filtered.
