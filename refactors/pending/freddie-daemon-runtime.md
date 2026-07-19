# the daemon is a shape, not a program

`crates/mercury/src/daemon.rs` is 320 lines, and about 30 of them are about remapping keys. The rest is the arrangement every freddie app needs: the main thread inside a run loop so `AppKit` can deliver callbacks, a menu-bar item whose Quit is a second way out, a worker thread holding a current-thread runtime, an event loop that dispatches into the model, an effect loop that performs what dispatch produced, and one log record per dispatch.

`freddie_daemon` is a new crate holding that arrangement. An app supplies its events, its effects, its model, and its sources; it gets the threads, the loops, the menu bar, and the dispatch record.

This is the other half of `freddie-cli.md`. That doc owns what the binary can be told; this one owns how the process is arranged once it has been told to be the daemon. They meet at one associated type.

## What is generic and what is not

Generic, and moving to `freddie_daemon`:

- main thread in `freddie_main_loop::main_loop`, and `init_menu_bar_app` before it
- the menu-bar item, its Quit, and the title channel the effect loop writes to
- the worker thread and its current-thread tokio runtime
- the event channel, the effect channel, and both loops
- the dispatch record: one `info!` per event carrying the event, its effects, and the resulting state
- `select!` over the two loops, so an effect can end the daemon
- SIGTERM, delivered as the same event the menu bar's Quit sends

The app's, and staying in mercury:

- what an event is, what an effect is, and what the model does with them
- its sources: the keyboard grab, the event socket, the app-navigation watcher
- performing an effect, which is the only place that knows what `Tap` or `Place` mean
- the icon

## The trait

```rust
/// A freddie model, arranged so the runtime can drive it.
///
/// The implementing value owns whatever the app's sources and effects need alive: a keyboard
/// grab, a socket, a watcher, an emitter. The runtime holds it for the length of the run and
/// drops it on the way out, so releasing those is a `Drop` impl rather than a shutdown step.
pub trait Daemon: Sized {
    /// What its sources produce and its model dispatches.
    type Event: fmt::Debug + Send + 'static;

    /// What dispatch produces and [`perform`](Self::perform) carries out.
    type Effect: fmt::Debug + Send + 'static;

    /// The menu-bar glyph: a black shape on transparency, rendered as a template.
    const ICON_PNG: &'static [u8];

    /// The event that asks the model to quit.
    ///
    /// The menu bar's Quit sends it, and so does SIGTERM. It goes through the model rather than
    /// ending the process, so whatever has to be undone first — a held modifier reopened, a grab
    /// released — is the model's own business and happens the same way however the ask arrived.
    fn stop_event() -> Self::Event;

    /// Anything that must happen on the main thread before the worker starts.
    ///
    /// `AppKit` reads that are main-thread-bound and cached for later belong here. Runs after
    /// `NSApp` exists and before the run loop is entered.
    fn init_main_thread() {}

    /// Build the model and start the sources, which push into `events`.
    ///
    /// Runs on the worker thread, inside the runtime, so a source may spawn. `None` means the
    /// daemon cannot run — the keyboard grab was refused, a port was busy — and the process exits
    /// without one. Say why on the terminal and in the log before returning it; the runtime knows
    /// only that there is nothing to run.
    fn start(events: &UnboundedSender<Self::Event>, menu_bar: &MenuBar) -> Option<Self>;

    /// Dispatch one event through the model, returning what it produced.
    fn handle(&mut self, event: &Self::Event) -> Vec<Self::Effect>;

    /// Perform one effect. `Break` ends the daemon and returns through `run`.
    fn perform(&mut self, effect: Self::Effect) -> ControlFlow<()>;
}
```

`MenuBar` is the handle the effect loop writes titles through:

```rust
/// The menu-bar item, as an app sees it from the worker thread.
///
/// An `NSStatusItem` is main-thread-only, so this is a sender rather than the item: the main
/// thread applies whatever arrives on its next wake. Only the last title in a batch is drawn,
/// because intermediate layers in one dispatch are not worth showing.
#[derive(Clone)]
pub struct MenuBar {
    titles: std::sync::mpsc::Sender<&'static str>,
}

impl MenuBar {
    /// Show `title` beside the glyph.
    ///
    /// Dropping the message is correct when the channel is closed: that means the main thread has
    /// gone, which the stopping path already handles.
    pub fn set_title(&self, title: &'static str) {
        let _ = self.titles.send(title);
    }
}
```

## The runtime

```rust
/// Be `D`: give the main thread to the run loop, and run the model on a worker.
///
/// `AppKit` delivers its callbacks only while the main thread is inside a run loop, so main sits
/// in one and the model runs elsewhere. See `refactors/past/main-thread.md`.
///
/// Dropping the worker's `Stopper` stops main's loop, so a normal return, a refused start, and a
/// panic all exit. Declaration order matters: the runtime drops before the `Stopper`.
pub fn run<D: Daemon>() -> i32 {
    freddie_main_loop::init_menu_bar_app();
    D::init_main_thread();

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    // Created here, not on the worker: the menu bar's Quit handler runs on THIS thread and needs a
    // sender, while the event loop on the worker owns the receiver.
    let (event_tx, event_rx) = unbounded_channel::<D::Event>();
    let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();

    let menu_bar = match freddie_menu_bar::show(D::NAME, D::ICON_PNG, {
        let event_tx = event_tx.clone();
        move || {
            let _ = event_tx.send(D::stop_event());
        }
    }) {
        Ok(bar) => bar,
        Err(e) => {
            eprintln!("menu bar: {e}");
            error!(error = %e, "could not create the menu bar");
            return 1;
        }
    };

    let worker = std::thread::Builder::new()
        .name("freddie-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(serve::<D>(event_tx, event_rx, MenuBar { titles: title_tx }))
        })
        .expect("spawning the runtime thread");

    main_loop.run(|| {
        if let Some(title) = title_rx.try_iter().last() {
            // The leading space is the gap between the glyph and the text, which the status item
            // does not put there itself.
            menu_bar.set_title(Some(&format!(" {title}")));
        }
    });
    let code = worker.join().unwrap_or(1);
    drop(menu_bar); // held until the loop returns, so the icon is up for the whole run
    code
}
```

`D::NAME` comes from `freddie_cli::App`, so `Daemon` is declared `pub trait Daemon: Sized` here and the runtime takes `D: Daemon + Named`, where `Named` is the one-const trait `freddie_cli::App` already satisfies. That keeps `freddie_daemon` from depending on the whole command line to learn a name.

```rust
/// Something with a name, which is what the menu bar's tooltip and the log records are keyed to.
pub trait Named {
    /// The app's name.
    const NAME: &'static str;
}
```

```rust
/// Everything the daemon does, on the worker thread.
///
/// `!Send` because an app's sources may hold main-thread-bound handles; it is `block_on`ed by the
/// worker's current-thread runtime and never crosses a thread.
#[expect(clippy::future_not_send)]
async fn serve<D: Daemon + Named>(
    event_tx: UnboundedSender<D::Event>,
    event_rx: UnboundedReceiver<D::Event>,
    menu_bar: MenuBar,
) -> i32 {
    let Some(daemon) = D::start(&event_tx, &menu_bar) else {
        return 1;
    };

    on_terminate::<D>(&event_tx);

    let (effect_tx, effect_rx) = unbounded_channel::<D::Effect>();

    // `select!` rather than `join!`: the effect loop ends on a `Break`, and the event loop never
    // does, because a source holds a sender for as long as it is alive.
    let mut daemon = daemon;
    tokio::select! {
        () = run_event_loop::<D>(&mut daemon, event_rx, effect_tx) => {}
        () = run_effect_loop::<D>(&mut daemon, effect_rx) => {}
    }
    0
}
```

The two loops and the record, generic over `D` and otherwise the bodies mercury has today:

```rust
/// The event loop: read the event channel and dispatch each event.
async fn run_event_loop<D: Daemon>(
    daemon: &mut D,
    mut event_rx: UnboundedReceiver<D::Event>,
    effect_tx: UnboundedSender<D::Effect>,
) {
    while let Some(event) = event_rx.recv().await {
        // One record per dispatch, carrying the event and the effects it produced, so a single
        // line tells the whole story of one event.
        let effects = daemon.handle(&event);
        info!(event = ?event, effects = ?effects, "dispatch");
        for effect in effects {
            let _ = effect_tx.send(effect);
        }
    }
}

/// The effect loop: perform each effect until one says to stop.
///
/// The one consumer of the effect channel, so effects are performed in the order dispatch produced
/// them: a modifier reaches the OS before the key carrying its flag.
async fn run_effect_loop<D: Daemon>(daemon: &mut D, mut effect_rx: UnboundedReceiver<D::Effect>) {
    while let Some(effect) = effect_rx.recv().await {
        if daemon.perform(effect).is_break() {
            break;
        }
    }
}
```

The dispatch record loses the state it carries today, because the runtime cannot format a model it does not know. `Daemon` gains nothing for it: mercury logs its own state inside `handle`, where it has it, and the runtime logs the event and the effects, which it has. The single line stays single.

## SIGTERM

```rust
/// Send `D::stop_event()` when the process is asked to terminate.
///
/// `launchctl bootout` and `<app> stop` both send SIGTERM. It goes through the model like every
/// other ask to quit, so a terminated process leaves the way it would have on its own.
///
/// A spawned task rather than a third `select!` arm, because an arm that completed would drop the
/// other two futures and skip the graceful path this exists to run.
fn on_terminate<D: Daemon>(event_tx: &UnboundedSender<D::Event>) {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: stopping");
                    let _ = event_tx.send(D::stop_event());
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated process will not stop gracefully");
        }
    }
}
```

`mercury-stop.md`'s change 1 is this, and `freddie-cli.md`'s `on_stop` helper is replaced by it: an app that declares `stop_event` never installs a handler itself.

Installing the handler replaces SIGTERM's default disposition, which the kernel honours unconditionally, with one that depends on the runtime being scheduled. A worker blocked inside a synchronous `perform` never completes `term.recv().await`, so such a process survives the `kill` it dies to today. `mercury-stop.md` records the measurement and ships `--force` as the answer.

## What mercury becomes

`crates/mercury/src/daemon.rs` keeps only what is mercury's: a struct holding the emitter, the interceptor, the socket, the watcher, and the model, and the `perform_effect` body as `perform`.

```rust
/// Mercury, running: the model and everything its sources and effects hold alive.
///
/// Dropping it releases the keyboard, closes the socket, and stops the watcher, so the way out
/// runs destructors whichever way the model was asked to quit.
pub(crate) struct MercuryDaemon {
    state: Mercury,
    emitter: Emitter,
    menu_bar: MenuBar,
    event_tx: UnboundedSender<MercuryEvent>,
    _interceptor: Interceptor,
    _socket: Socket,
    _watcher: Watcher,
}

impl Daemon for MercuryDaemon {
    type Event = MercuryEvent;
    type Effect = MercuryEffect;

    const ICON_PNG: &'static [u8] = include_bytes!("../assets/mercury.png");

    fn stop_event() -> MercuryEvent {
        mercury::quit_event()
    }

    fn init_main_thread() {
        // `freddie_windows` reads the screen's visible frame, which is AppKit and so
        // main-thread-bound. Do it while we are one, and cache it.
        if let Err(e) = freddie_windows::init() {
            eprintln!("windows: {e}");
            error!(error = %e, "window placement unavailable");
        }
    }

    fn start(events: &UnboundedSender<MercuryEvent>, menu_bar: &MenuBar) -> Option<Self> { .. }

    fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> {
        let effects = self.state.handle(event).unwrap_or_default();
        info!(state = ?self.state, "state");
        effects
    }

    fn perform(&mut self, effect: MercuryEffect) -> ControlFlow<()> { .. }
}
```

`start` is today's `serve` up to the `select!`: bind the socket, grab the keyboard, seed the frontmost app, install the watcher, build `Mercury::default()` with the front app set, and send the boot layer's name to the menu bar. Its failure arms return `None` after the `eprintln!` and `error!` they already write.

`perform` is today's `perform_effect` with the free functions it calls — `schedule_timer`, `place_window`, `foreground_app` — moving with it, and `title_tx.send(name)` becoming `self.menu_bar.set_title(name)`.

## The changes, in order

1. **`freddie_daemon` with the trait and the runtime**, and mercury implementing it. `freddie-cli.md`'s change 1 lands first, so the lock and the logging are already out of `daemon.rs` and `run` has one thing left to become.
2. **`Named` and the icon**, folding `freddie_cli::App::NAME` and the menu bar's tooltip into one source.

`freddie_cli::App` then reads:

```rust
pub trait App: Named {
    type Args: clap::Args + fmt::Debug;
    type Daemon: Daemon + Named;
    const ABOUT: &'static str;
}
```

with `App::run` gone: `freddie_cli`'s daemon verb calls `freddie_daemon::run::<A::Daemon>()` once the lock is held, and an app writes no function to be the daemon at all.
