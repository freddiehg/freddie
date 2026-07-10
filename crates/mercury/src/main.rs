//! Mercury's runnable v1: grab the keyboard and drive the model.
//!
//! `freddie_keyboard::intercept` swallows every key and hands each to the model.
//! Two loops over two channels: the event loop dispatches (mutating state,
//! producing effects); the effect loop performs them, re-emitting keys through the
//! `Emitter` and foregrounding apps through `freddie_app_nav`. A
//! `freddie_app_nav` watcher runs the app-navigation source: it reports the
//! frontmost app as a `Foreground` event, so foregrounding an app (the effect) and
//! observing it come up (the event) are decoupled the way the model expects.
//!
//! Every key goes through the model, so `escape` is handled there (it goes home)
//! and `q` from home quits. A 30-second timer is the backstop out of the hijack
//! (hard exit 5s after that).
//!
//! The `Emitter` is `!Send`, so the effect loop runs on this task via `join!`
//! rather than a spawned task.
//!
//! # Threads
//!
//! Three, each asleep in its own loop, joined only by a channel.
//!
//! The main thread runs the platform run loop and nothing else, so that `AppKit`
//! can deliver callbacks there. It is a doorman: a callback sends into the event
//! channel and returns. Main-thread callbacks are serialized, so a slow one would
//! stall every other source.
//!
//! The tap thread, spawned inside `freddie_keyboard::intercept`, runs its own run
//! loop for the `CGEventTap`. It has always been off main, which is why the
//! keyboard works whatever main is doing.
//!
//! The worker thread runs the tokio runtime, owns the state and the `!Send`
//! `Emitter`, and runs both the event and effect loops. It is the only place
//! state is mutated, so there is no shared mutable state and no `Mutex`.
//!
//! On macOS this needs Accessibility (and Input Monitoring). `cargo run -p mercury`

use std::path::PathBuf;
use std::time::Duration;

use freddie_keyboard::Emitter;
use mercury::{App, Mercury, MercuryEffect, MercuryEvent, foreground};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, error, info, warn};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// The log file, written under [`log_dir`].
const LOG_FILE: &str = "mercury.log";

/// The environment variable that sets the terminal's log level.
const LOG_LEVEL_ENV: &str = "LOG_LEVEL";

/// What the log file records, always. Deliberately not tied to [`LOG_LEVEL_ENV`]:
/// the file is the record of what happened, so quieting the terminal must never
/// quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Give the main thread to the run loop, and run mercury on a worker thread.
///
/// The main thread does nothing here, forever, and that is the point. `AppKit`
/// delivers its callbacks on the main thread's run loop and only while a thread
/// is inside it, so anything that wants to observe `NSWorkspace` or own an
/// `NSStatusItem` needs main to be sitting in [`freddie_main_loop::MainLoop::run`]
/// rather than running our code. See `refactors/pending/main-thread.md`.
///
/// That forces the inversion below. The tokio runtime cannot also live on main,
/// because `block_on` is itself a loop that wants to own the thread's sleep, and
/// a thread can only be blocked in one syscall. So everything mercury does moves
/// to a worker thread, and [`run`] is what used to be the body of `main`.
///
/// The worker holds the [`Stopper`](freddie_main_loop::Stopper). Dropping it
/// stops main's run loop, which is how the process exits: whether [`run`] returns
/// normally, returns early because the keyboard could not be grabbed, or panics
/// and unwinds, the `Stopper` goes with it and main falls out of its loop. Note
/// the declaration order in the closure, which is load-bearing: the runtime is
/// dropped before the `Stopper`, so the loop is not stopped until the runtime is
/// gone.
///
/// The keyboard tap is unaffected by any of this. `intercept` spawns its own
/// thread and adds its source to that thread's run loop, which is why the tap
/// works today with tokio on main, and why it keeps working with tokio off it.
fn main() {
    let log_path = init_tracing();
    println!("mercury: logging to {}", log_path.display());

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    let worker = std::thread::Builder::new()
        .name("mercury-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(run());
        })
        .expect("spawning the runtime thread");

    main_loop.run(); // services AppKit sources until the worker drops the stopper
    let _ = worker.join();
}

/// Everything mercury does, on the worker thread.
///
/// `intercept` has to be called from here rather than from `main`, because it
/// returns the `Emitter`, the `Emitter` is `!Send` (it holds an `Rc`), and the
/// effect loop uses it. It has to be born on the thread it will live on.
///
/// Which is exactly why this future is `!Send`, and why that is fine: it is
/// `block_on`ed by the worker's current-thread runtime and never crosses a
/// thread.
#[allow(clippy::future_not_send)]
async fn run() {
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // Grab the keyboard: swallow every key and forward it to the model, which
    // decides what to emit (the effect loop performs it).
    let grabbed = freddie_keyboard::intercept({
        let event_tx = event_tx.clone();
        move |ev| {
            // Forward every key, down and up, with its real press. Dropping the up
            // leaves a modifier stuck down in the emitted stream: after ctrl-a, a
            // swallowed ctrl-up means the next key still carries ctrl (p arrives as
            // ctrl-p). We always swallow here; the model dispatches and the effect
            // loop re-emits whatever passes through.
            let _ = event_tx.send(MercuryEvent::Key(ev));
            None
        }
    });
    let (interceptor, emitter) = match grabbed {
        Ok(pair) => pair,
        Err(e) => {
            // Usually Accessibility is not granted. Say so on the terminal too: the
            // user is looking there, not at the log file.
            eprintln!("keyboard: {e}");
            error!(error = %e, "could not intercept the keyboard");
            return;
        }
    };

    // The app-navigation source: report the frontmost app as a foreground event.
    let _watcher = freddie_app_nav::watch(freddie_app_nav::DEFAULT_POLL_INTERVAL, {
        let event_tx = event_tx.clone();
        move |name| {
            let _ = event_tx.send(foreground(App::from_name(name)));
        }
    });

    spawn_killswitch(effect_tx.clone());

    println!("mercury: hijacking the keyboard; escape then q quits (30s backstop)");
    info!("hijacking the keyboard; escape then q quits (30s backstop)");
    tokio::join!(
        run_event_loop(Mercury::default(), event_rx, effect_tx),
        run_effect_loop(effect_rx, emitter),
    );
    drop(interceptor); // hold the grab until here
}

/// Where the log file lives: the macOS per-user log directory, or the current
/// directory when `HOME` is unset.
fn log_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("."),
        |home| PathBuf::from(home).join("Library/Logs/mercury"),
    )
}

/// Send tracing to the log file and the terminal, and return the file's path.
///
/// Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
/// the record of a run survives however quiet the terminal was asked to be. The
/// terminal shows whatever `LOG_LEVEL` asks for, defaulting to `info`.
///
/// The file appender writes synchronously rather than through
/// `tracing_appender`'s non-blocking writer: mercury exits via `process::exit`,
/// which runs no destructors, so a buffered writer would drop whatever it had not
/// flushed.
fn init_tracing() -> PathBuf {
    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("mercury: could not create {}: {e}", dir.display());
    }

    let file = fmt::layer()
        .with_writer(tracing_appender::rolling::never(&dir, LOG_FILE))
        .with_ansi(false)
        .with_filter(FILE_LEVEL);

    let terminal = fmt::layer().with_writer(std::io::stderr).with_filter(
        EnvFilter::try_from_env(LOG_LEVEL_ENV).unwrap_or_else(|_| EnvFilter::new("info")),
    );

    tracing_subscriber::registry()
        .with(file)
        .with(terminal)
        .init();
    dir.join(LOG_FILE)
}

/// Dev killswitch: a `Kill` effect after 30s (the effect loop exits on it), then a
/// hard exit 5s later if that never happened.
fn spawn_killswitch(effect_tx: UnboundedSender<MercuryEffect>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let _ = effect_tx.send(MercuryEffect::Kill);
        tokio::time::sleep(Duration::from_secs(5)).await;
        std::process::exit(1);
    });
}

/// The event loop: read the event channel and dispatch each event.
async fn run_event_loop(
    mut state: Mercury,
    mut event_rx: UnboundedReceiver<MercuryEvent>,
    effect_tx: UnboundedSender<MercuryEffect>,
) {
    info!(state = ?state, "initial state");
    while let Some(event) = event_rx.recv().await {
        dispatch_event(&mut state, &event, &effect_tx);
    }
}

/// Dispatch one event through freddie and enqueue whatever effects it produced.
///
/// One record per dispatch, carrying the event, the effects it produced, and the
/// state it left behind, so a single line tells the whole story of one event.
fn dispatch_event(
    state: &mut Mercury,
    event: &MercuryEvent,
    effect_tx: &UnboundedSender<MercuryEffect>,
) {
    let effects = state.handle(event).unwrap_or_default();
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}

/// The effect loop: read the effect channel and perform each effect.
///
/// The `Emitter` is `!Send` by design, so this future is `!Send`; `main` runs it
/// on the current thread via `join!` and it never crosses a thread.
#[allow(clippy::future_not_send)]
async fn run_effect_loop(mut effect_rx: UnboundedReceiver<MercuryEffect>, emitter: Emitter) {
    while let Some(effect) = effect_rx.recv().await {
        perform_effect(&effect, &emitter);
    }
}

/// Perform one effect: emit keys, foreground an app, or exit. The effect itself is
/// already on the dispatch record; these are the performance details.
fn perform_effect(effect: &MercuryEffect, emitter: &Emitter) {
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(*app),
        MercuryEffect::Emit(ke) => match emitter.emit(ke.key, ke.press) {
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
            Err(e) => warn!(key = ?ke.key, press = ?ke.press, error = %e, "emit failed"),
        },
        MercuryEffect::Kill => {
            info!("kill: exiting");
            std::process::exit(0);
        }
    }
}

/// Foreground an app for real, fire-and-forget on its own thread so the effect
/// loop never blocks on `open`. The watcher reports the app that actually comes
/// up, so nothing here waits on the result (see `app-foregrounding.md`).
fn foreground_app(app: App) {
    let Some(name) = app.launch_name() else {
        warn!(app = ?app, "no launch name; not foregrounding");
        return;
    };
    std::thread::spawn(move || match freddie_app_nav::foreground(name) {
        Ok(()) => debug!(app = name, "foregrounded"),
        Err(e) => warn!(app = name, error = %e, "foreground failed"),
    });
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use mercury::{Key, key};
    use tracing_subscriber::fmt::MakeWriter;

    use super::{
        EnvFilter, FILE_LEVEL, LOG_FILE, Layer, Mercury, SubscriberExt, dispatch_event, log_dir,
        unbounded_channel,
    };

    /// A writer that collects everything the subscriber emits.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().expect("not poisoned").clone()).expect("utf8")
        }
    }

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("not poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// One dispatch is one record, carrying the event, the effects, and the state
    /// it left behind. Regression test for the log being split across lines.
    #[test]
    fn dispatch_logs_event_effects_and_state_on_one_line() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .finish();

        let (effect_tx, _effect_rx) = unbounded_channel();
        let mut state = Mercury::default();
        tracing::subscriber::with_default(subscriber, || {
            dispatch_event(&mut state, &key(Key::KeyN), &effect_tx);
        });

        let out = buffer.contents();
        let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "expected exactly one record, got: {out}");
        let line = lines[0];
        assert!(line.contains("event="), "no event field: {line}");
        assert!(line.contains("effects="), "no effects field: {line}");
        assert!(line.contains("state="), "no state field: {line}");
        // `n` from home enters nav, so the state on the record is the state after.
        assert!(
            line.contains("Nav"),
            "state is the post-dispatch state: {line}"
        );
    }

    /// The file appender writes as each record is emitted, with no flush and no
    /// guard. That is what lets `Kill`'s `process::exit` (which runs no
    /// destructors) leave a complete log behind.
    #[test]
    fn the_file_appender_writes_without_a_flush() {
        let dir = std::env::temp_dir().join(format!("mercury-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let subscriber = tracing_subscriber::fmt()
            .with_writer(tracing_appender::rolling::never(&dir, LOG_FILE))
            .with_ansi(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(marker = "written-synchronously", "dispatch");
        });

        // Read it back without having dropped or flushed anything.
        let logged = std::fs::read_to_string(dir.join(LOG_FILE)).expect("log file exists");
        std::fs::remove_dir_all(&dir).ok();
        assert!(logged.contains("written-synchronously"), "{logged}");
    }

    /// `LOG_LEVEL` governs the terminal only. However quiet the terminal is asked
    /// to be, the file still records down to [`FILE_LEVEL`].
    #[test]
    fn the_log_level_quiets_the_terminal_but_never_the_file() {
        let to_file = Buffer::default();
        let to_terminal = Buffer::default();

        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(to_file.clone())
                    .with_ansi(false)
                    .with_filter(FILE_LEVEL),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(to_terminal.clone())
                    .with_ansi(false)
                    // The terminal is silenced short of `error`.
                    .with_filter(EnvFilter::new("error")),
            );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(marker = "an-info-record", "dispatch");
            tracing::debug!(marker = "a-debug-record", "emitted");
        });

        let file = to_file.contents();
        assert!(file.contains("an-info-record"), "file lost info: {file}");
        assert!(file.contains("a-debug-record"), "file lost debug: {file}");
        assert!(
            to_terminal.contents().is_empty(),
            "terminal should be silent: {}",
            to_terminal.contents()
        );
    }

    #[test]
    fn log_dir_is_under_the_user_log_directory() {
        // Only meaningful with HOME set, which it is under cargo test.
        if std::env::var_os("HOME").is_some() {
            let path = log_dir().join(LOG_FILE);
            assert!(
                path.ends_with("Library/Logs/mercury/mercury.log"),
                "{path:?}"
            );
        }
    }
}
