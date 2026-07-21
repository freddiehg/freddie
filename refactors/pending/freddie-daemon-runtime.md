# the daemon is a shape, not a program

`crates/mercury/src/daemon.rs` is 454 lines, and about 80 of them are about remapping keys. The rest is the arrangement every freddie app needs: the main thread inside a run loop so `AppKit` can deliver callbacks, a menu-bar item whose Quit is a second way out, a worker thread holding a current-thread runtime, an event loop that dispatches into the model, an effect loop that performs what dispatch produced, and one log record per dispatch.

`freddie_daemon` is a new crate holding that arrangement. An app supplies its events, its effects, its model, and its sources; it gets the threads, the loops, the menu bar, and the dispatch record.

This is the other half of `freddie-cli.md`. That doc owns what the binary can be told; this one owns how the process is arranged once it has been told to be the daemon. They meet at one associated type.

## What is generic and what is not

Generic, and moving to `freddie_daemon`:

- main thread in `freddie_main_loop::main_loop`, and `init_menu_bar_app` before it
- the menu-bar item, its Quit, and the title channel the effect loop writes to
- the worker thread and its current-thread tokio runtime
- the event channel, the effect channel, and the loop over both
- the dispatch record: one `info!` per event carrying the event and the effects it produced
- SIGTERM, delivered as the same event the menu bar's Quit sends

The app's, and staying in mercury:

- what an event is, what an effect is, and what the model does with them
- its sources: the keyboard grab, the event socket, the app-navigation watcher
- performing an effect, which is the only place that knows what `Tap` or `Place` mean
- the icon and the menu bar's title

## The trait

```rust
/// A freddie model, arranged so the runtime can drive it.
///
/// The implementing value owns whatever the app's sources and effects need alive: a keyboard
/// grab, a socket, a watcher, an emitter. The runtime holds it for the length of the run and
/// drops it on the way out, so releasing those is a `Drop` impl rather than a shutdown step.
pub trait Daemon: Sized {
    /// What its sources produce and its model dispatches.
    ///
    /// `From<Stop>` because the runtime has to ask this model to quit without knowing anything
    /// else about its vocabulary. The bound sits on the event rather than on the daemon: it is the
    /// event type that has to be able to say it, and the runtime needs one built before any daemon
    /// value exists, for the menu bar's Quit closure.
    type Event: fmt::Debug + Send + From<Stop> + 'static;

    /// What dispatch produces and [`perform`](Self::perform) carries out.
    type Effect: fmt::Debug + Send + 'static;

    /// Whatever [`start`](Self::start) needs that came from the command line.
    ///
    /// `()` for an app whose daemon takes no flags. Unbounded, and deliberately not a `clap` type:
    /// this crate arranges a process and never parses one's arguments. `freddie_cli::App` is what
    /// turns flags into one of these.
    type Config;

    /// The menu-bar glyph: a black shape on transparency, rendered as a template.
    const ICON_PNG: &'static [u8];

    /// The menu-bar item's tooltip, which is the app's name as a person reads it.
    ///
    /// Its own const rather than `freddie_cli::App::NAME`, because that name keys a lock file and
    /// a launchd label and so is lowercase, and this is display text.
    const TITLE: &'static str;

    /// Anything that must happen on the main thread before the worker starts.
    ///
    /// `AppKit` reads that are main-thread-bound and cached for later belong here. Runs after
    /// `NSApp` exists and before the run loop is entered.
    fn init_main_thread() {}

    /// Build the model and start the sources, which push into `events`.
    ///
    /// Runs on the worker thread, inside the runtime, so a source may spawn. `None` means the
    /// daemon cannot run: the keyboard grab was refused, a port was busy. Say why in the log
    /// before returning it; the runtime knows only that there is nothing to run.
    fn start(
        config: &Self::Config,
        events: &UnboundedSender<Self::Event>,
        menu_bar: &MenuBar,
    ) -> Option<Self>;

    /// Dispatch one event through the model, returning what it produced.
    fn handle(&mut self, event: &Self::Event) -> Vec<Self::Effect>;

    /// Perform one effect. `Break` ends the daemon and returns through `run`.
    fn perform(&mut self, effect: Self::Effect) -> ControlFlow<()>;
}
```

`Stop` is the one thing the runtime says to a model it otherwise knows nothing about:

```rust
/// The ask to quit, which the menu bar's Quit and SIGTERM both deliver.
///
/// It goes through the model rather than ending the process, so whatever has to be undone first,
/// a held modifier reopened or a grab released, is the model's own business, and happens the same
/// way however the ask arrived.
pub struct Stop;
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
pub fn run<D: Daemon>(config: D::Config) -> i32
where
    D::Config: Send + 'static,
{
    freddie_main_loop::init_menu_bar_app();
    D::init_main_thread();

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    // Created here, not on the worker: the menu bar's Quit handler runs on THIS thread and needs a
    // sender, while the event loop on the worker owns the receiver.
    let (event_tx, event_rx) = unbounded_channel::<D::Event>();
    let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();

    let menu_bar = match freddie_menu_bar::show(D::TITLE, D::ICON_PNG, {
        let event_tx = event_tx.clone();
        move || {
            let _ = event_tx.send(Stop.into());
        }
    }) {
        Ok(bar) => bar,
        Err(e) => {
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
            runtime.block_on(serve::<D>(&config, event_tx, event_rx, MenuBar { titles: title_tx }))
        })
        .expect("spawning the runtime thread");

    // Pumps AppKit events until the worker drops the stopper, applying any pending title on each
    // wake. The leading space is the gap between the glyph and the text, which the status item
    // does not put there itself.
    main_loop.run(|| {
        if let Some(title) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {title}")));
        }
    });
    let code = worker.join().unwrap_or(1);
    drop(menu_bar); // held until the loop returns, so the icon is up for the whole run
    code
}
```

```rust
/// Everything the daemon does, on the worker thread.
///
/// `!Send` because an app's sources may hold main-thread-bound handles; it is `block_on`ed by the
/// worker's current-thread runtime and never crosses a thread.
#[expect(clippy::future_not_send)]
async fn serve<D: Daemon>(
    config: &D::Config,
    event_tx: UnboundedSender<D::Event>,
    event_rx: UnboundedReceiver<D::Event>,
    menu_bar: MenuBar,
) -> i32 {
    let Some(mut daemon) = D::start(config, &event_tx, &menu_bar) else {
        return 1;
    };

    on_terminate::<D>(&event_tx);

    let (effect_tx, effect_rx) = unbounded_channel::<D::Effect>();
    run_loop::<D>(&mut daemon, event_rx, effect_tx, effect_rx).await;
    0
}
```

## One loop, not two

The two loops mercury runs today become one, because they now drive one value. `handle` and `perform` both take `&mut D`, and two futures in a `select!` cannot each hold that borrow. One loop taking the borrow per iteration is what compiles, and it is also what the ordering wants.

```rust
/// Dispatch events and perform what they produce, until an effect says to stop.
///
/// `biased`, so a pending effect is always performed before the next event is dispatched: a
/// modifier reaches the OS before the key carrying its flag. The effect channel is drained to
/// empty between events, since a dispatch's effects are all queued before this returns to the
/// event arm.
async fn run_loop<D: Daemon>(
    daemon: &mut D,
    mut event_rx: UnboundedReceiver<D::Event>,
    effect_tx: UnboundedSender<D::Effect>,
    mut effect_rx: UnboundedReceiver<D::Effect>,
) {
    loop {
        tokio::select! {
            biased;

            Some(effect) = effect_rx.recv() => {
                if daemon.perform(effect).is_break() {
                    break;
                }
            }

            Some(event) = event_rx.recv() => {
                // One record per dispatch, carrying the event and the effects it produced, so a
                // single line tells the whole story of one event.
                let effects = daemon.handle(&event);
                info!(event = ?event, effects = ?effects, "dispatch");
                for effect in effects {
                    let _ = effect_tx.send(effect);
                }
            }

            // Both channels closed. `effect_tx` lives here, so this is unreachable while the loop
            // runs; `select!` panics without it rather than taking the branch.
            else => break,
        }
    }
}
```

The dispatch record loses the state it carries today, because the runtime cannot format a model it does not know. `Daemon` gains nothing for it: mercury logs its own state inside `handle`, where it has it, and the runtime logs the event and the effects, which it has. The single line stays single.

## SIGTERM

```rust
/// Send [`Stop`] into the model when the process is asked to terminate.
///
/// `launchctl bootout` and `<app> stop` both send SIGTERM. It goes through the model like every
/// other ask to quit, so a terminated process leaves the way it would have on its own.
///
/// A spawned task rather than a third `select!` arm, because an arm that completed would drop the
/// other futures and skip the graceful path this exists to run.
fn on_terminate<D: Daemon>(event_tx: &UnboundedSender<D::Event>) {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: stopping");
                    let _ = event_tx.send(Stop.into());
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated process will not stop gracefully");
        }
    }
}
```

mercury's own handler, which this replaces, is deleted from `serve`; its `From<Stop>` impl is what remains.

Installing the handler replaces SIGTERM's default disposition, which the kernel honours unconditionally, with one that depends on the runtime being scheduled. A worker blocked inside a synchronous `perform` never completes `term.recv().await`, so such a process survives the `kill` it dies to today. `refactors/past/mercury-stop.md` records the measurement, and `--force` is the answer.

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
    /// The loopback port the event socket listens on.
    type Config = u16;

    const ICON_PNG: &'static [u8] = include_bytes!("../assets/mercury.png");
    const TITLE: &'static str = "Mercury";

    fn init_main_thread() {
        // `freddie_windows` reads the screen's visible frame, which is AppKit and so
        // main-thread-bound. Do it while we are one, and cache it.
        if let Err(e) = freddie_windows::init() {
            error!(error = %e, "window placement unavailable");
        }
    }

    fn start(
        port: &u16,
        events: &UnboundedSender<MercuryEvent>,
        menu_bar: &MenuBar,
    ) -> Option<Self> { .. }

    fn handle(&mut self, event: &MercuryEvent) -> Vec<MercuryEffect> {
        let effects = self.state.handle(event).unwrap_or_default();
        info!(state = ?self.state, "state");
        effects
    }

    fn perform(&mut self, effect: MercuryEffect) -> ControlFlow<()> { .. }
}
```

mercury spells the ask in its own vocabulary, which it already has a variant for:

```rust
impl From<Stop> for MercuryEvent {
    fn from(_: Stop) -> Self {
        quit_event()
    }
}
```

`start` is today's `serve` up to the `select!`: bind the socket, grab the keyboard, seed the frontmost app, install the watcher, build `Mercury::default()` with the front app set, and send the boot layer's name to the menu bar. Its failure arms return `None` after the `error!` they already write; nothing is printed, per `refactors/past/one-log-many-writers.md`.

`perform` is today's `perform_effect` with the free functions it calls, `schedule_timer`, `place_window`, `copy`, and `foreground_app`, moving with it, and `title_tx.send(name)` becoming `self.menu_bar.set_title(name)`.

## Where the port comes from

`freddie_cli::App` maps the flags it parsed onto the config its daemon starts from, so `freddie_daemon` never sees a `clap` type and mercury never repeats the port.

```rust
pub trait App {
    type Args: clap::Args + fmt::Debug;
    type Daemon: Daemon;

    const NAME: &'static str;
    const ABOUT: &'static str;

    /// What this app's daemon needs from the flags it was given.
    fn config(args: &Self::Args) -> <Self::Daemon as Daemon>::Config;
}
```

`App::run` is gone: the daemon verb calls `freddie_daemon::run::<A::Daemon>(A::config(&args.app))` once the lock is held, and an app writes no function to be the daemon at all.

`crates/mercury/src/main.rs`, entire:

```rust
//! The mercury binary: a freddie app and the command line that runs it.

use freddie_cli::App;

use daemon::MercuryDaemon;

mod daemon;

/// The loopback port the event socket listens on, and nothing else mercury needs from a flag.
#[derive(clap::Args, Debug)]
pub struct MercuryArgs {
    /// The loopback port the event socket listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}

struct Mercury;

impl App for Mercury {
    type Args = MercuryArgs;
    type Daemon = MercuryDaemon;

    const NAME: &'static str = "mercury";
    const ABOUT: &'static str = "A layered keyboard remapper.";

    fn config(args: &MercuryArgs) -> u16 {
        args.port
    }
}

fn main() -> ! {
    freddie_cli::main::<Mercury>()
}
```

## The changes, in order

`freddie-cli.md` lands first, so the lock and the logging are already out of `daemon.rs` and `run` has one thing left to become.

1. **`freddie_daemon` with the trait and the runtime**, and mercury implementing it. `freddie_cli::App` keeps `run`, and mercury's is `freddie_daemon::run::<MercuryDaemon>(args.port)`.
2. **`App::Daemon` replaces `App::run`**, folding the two traits together at `config`, so the daemon verb reaches `freddie_daemon::run` itself and mercury's `main.rs` is the file above.
