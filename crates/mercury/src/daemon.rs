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
//! and `q` from home quits. The menu bar's Quit is a second way out, one that does
//! not depend on the grabbed keyboard still working.
//!
//! Quitting is a `Kill` effect, which ends the effect loop rather than exiting the
//! process, so the way out runs destructors: the keyboard is released and main's
//! run loop is stopped.
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
//! The worker thread runs the tokio runtime, owns the state and the `Emitter`, and runs both
//! the event and effect loops. It is the only place state is mutated, so there is no shared
//! mutable state and no `Mutex`, and it is the only consumer of the effect channel, so effects
//! are performed in the order dispatch produced them: a modifier reaches the OS before the key
//! carrying its flag.
//!
//! On macOS this needs Accessibility (and Input Monitoring). `cargo run -p mercury`

use std::ops::ControlFlow;

use freddie::{AlwaysEqual, TimerEffect};
use freddie_keyboard::Emitter;
use freddie_overlay::OverlaySink;
use freddie_windows::{WindowFrame, WindowSink};
use mercury::{
    App, Chord, Copied, Mercury, MercuryEffect, MercuryEvent, UrlPart, WindowEvent, Windows,
    foreground, host, quit_event,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot::error::TryRecvError;
use tracing::{debug, error, info, warn};

/// Be the daemon: give the main thread to the run loop, and run mercury on a worker thread.
///
/// `AppKit` delivers its callbacks only while the main thread is inside a run
/// loop, so main sits in one and mercury runs in [`serve`] elsewhere. See
/// `refactors/past/main-thread.md`.
///
/// Dropping the worker's `Stopper` stops main's loop, so a normal return, a
/// failed keyboard grab, and a panic all exit. Declaration order below matters:
/// the runtime drops before the `Stopper`.
pub(crate) fn run(port: u16) {
    // NSApp as an accessory (menu-bar) app, before the status item is created and
    // before the loop pumps its events.
    freddie_main_loop::init_menu_bar_app();

    let (main_loop, stopper) = freddie_main_loop::main_loop();

    // The event channel is created here, not in `run`: the menu bar's Quit handler
    // runs on THIS (main) thread and needs a sender, while the event loop on the
    // worker owns the receiver.
    let (event_tx, event_rx) = unbounded_channel::<MercuryEvent>();

    // Titles for the status item. The effect loop, on the worker, sends; the main thread applies
    // them on its next wake, because an NSStatusItem is main-thread-only. A std channel rather
    // than tokio's: the receiving end is the main thread, which is not in the runtime.
    let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();

    // The status item, on the main thread now that NSApp exists. A Quit click
    // enqueues the same kind of event any source does; the model turns it into
    // `Kill`, which ends the effect loop, releases the keyboard, and drops the
    // stopper. So Quit is the mouse-reachable way out even if the grabbed keyboard
    // is wedged.
    let menu_bar = freddie_menu_bar::show("Mercury", include_bytes!("../assets/mercury.png"), {
        let event_tx = event_tx.clone();
        move || {
            let _ = event_tx.send(quit_event());
        }
    });
    let menu_bar = match menu_bar {
        Ok(bar) => bar,
        Err(e) => {
            error!(error = %e, "could not create the menu bar");
            return;
        }
    };

    // The overlay panel, built here because `NSPanel` is main-thread-only, and held for the
    // life of `main` like `menu_bar`: dropping it closes the panel.
    let overlay = freddie_overlay::overlay();

    // The app-navigation source, installed before the seed below is read: an app switch
    // between the two is then queued as an event rather than lost, and the model converges.
    // Delivery is on this thread either way. Held for the life of `main`, like `menu_bar`,
    // because dropping it deregisters.
    let _app_watcher = freddie_app_nav::watch({
        let event_tx = event_tx.clone();
        move |bundle_id| {
            let _ = event_tx.send(foreground(App::from_bundle_id(bundle_id)));
        }
    });

    // The window source. Here rather than in `serve` because a `Watcher` is `!Send` and its
    // observers register against this thread's run loop. Installed before its snapshot is
    // taken, which `watch` guarantees by returning both.
    let windows = freddie_windows::watch({
        let event_tx = event_tx.clone();
        move |change| {
            let _ = event_tx.send(MercuryEvent::Window(WindowEvent { change }));
        }
    });
    let (_window_watcher, window_sink, window_state) = match windows {
        Ok((watcher, snapshot)) => {
            let sink = watcher.sink();
            (Some(watcher), Some(sink), Windows::from_snapshot(snapshot))
        }
        Err(e) => {
            error!(error = %e, "window observation unavailable");
            (None, None, Windows::default())
        }
    };

    // Everything read from the OS before the main loop turns. After `main_loop.run`, every
    // fact reaches the model as an event; this is the only other way in.
    let boot = Boot {
        front_app: freddie_app_nav::frontmost()
            .map_or(App::Other, |bundle_id| App::from_bundle_id(&bundle_id)),
        windows: window_state,
        window_sink,
        overlay: overlay.sink(),
    };

    let worker = std::thread::Builder::new()
        .name("mercury-runtime".to_owned())
        .spawn(move || {
            let _stopper = stopper; // dropped last: see the note above
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime with no reactor cannot fail to build");
            runtime.block_on(serve(boot, event_tx, event_rx, title_tx, port));
        })
        .expect("spawning the runtime thread");

    // Pumps AppKit events until the worker drops the stopper, applying any pending title on each
    // wake. Only the last one is drawn: intermediate layers in one batch are not worth showing.
    // The leading space is the gap between the glyph and the text, which the status item does not
    // put there itself.
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(&format!(" {name}")));
        }
    });
    let _ = worker.join();
    // Held until the loop returns, so the icon is up and the panel is available for the whole run.
    drop(menu_bar);
    drop(overlay);
}

/// What the process read from the OS before the main loop started turning.
///
/// Reading the OS is allowed while this is being built and at no point after: once
/// `main_loop.run` is going, every fact reaches the model as an event, so anything the
/// model needs to start from has to be in here.
struct Boot {
    /// The app that was already frontmost. `freddie_app_nav::watch` reports changes, and at
    /// boot nothing has changed yet.
    front_app: App,
    /// Every window open when the watcher was installed, which one was focused, and the
    /// screens. Same reasoning: the observer reports changes, and none has happened yet.
    windows: Windows,
    /// The handle placements are performed through. `None` when window observation could
    /// not start, in which case a placement has nothing to act on and says so.
    window_sink: Option<WindowSink>,
    /// The handle the overlay is shown and hidden through.
    overlay: OverlaySink,
}

/// Everything mercury does, on the worker thread.
///
/// `intercept` is called from here rather than from `main` because the tap and the effect loop
/// belong with the state they drive, not because anything it returns is pinned to a thread.
async fn serve(
    boot: Boot,
    event_tx: UnboundedSender<MercuryEvent>,
    event_rx: UnboundedReceiver<MercuryEvent>,
    title_tx: std::sync::mpsc::Sender<&'static str>,
    port: u16,
) {
    let (effect_tx, effect_rx) = unbounded_channel::<MercuryEffect>();

    // The external event source, held for the length of `run` like `_watcher`: dropping it closes
    // the port. Above the keyboard grab, so a refused start has not taken the keyboard yet.
    //
    // A busy port panics. The single-instance lock already means the squatter is some other
    // program, and a mercury that came up deaf would present as "the extension broke" while
    // looking perfectly healthy.
    let _socket = freddie_event_socket::listen(port, {
        let event_tx = event_tx.clone();
        move |text| mercury::on_message(text, &event_tx)
    })
    .unwrap_or_else(|e| {
        panic!("could not bind 127.0.0.1:{port}: {e}; find it with `lsof -i :{port}`")
    });

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
            // Usually Accessibility is not granted.
            error!(error = %e, "could not intercept the keyboard");
            return;
        }
    };

    // `launchctl bootout` and `mercury stop` both send SIGTERM. Route it into the event channel as
    // the same Quit the menu bar sends, so a terminated mercury leaves the way it would have on
    // its own: the model turns it into `Kill`, the effect loop breaks, and the `Interceptor`
    // releases the keyboard.
    //
    // A spawned task rather than a third `select!` arm, because an arm that completed would drop
    // the other two futures and skip the graceful path this exists to run.
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut term) => {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if term.recv().await.is_some() {
                    info!("SIGTERM: quitting");
                    let _ = event_tx.send(quit_event());
                }
            });
        }
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; a terminated mercury will not release the keyboard");
        }
    }

    // `select!` rather than `join!`: the effect loop ends on `Kill`, and the event
    // loop never does, because the tap thread holds a sender for as long as the
    // grab is alive.
    let mercury = Mercury::new(boot.front_app, boot.windows);

    // Nothing has transitioned yet, so no `ShowLayer` has been produced: name the layer we boot
    // into, or the item stays blank until the first layer change.
    let _ = title_tx.send(mercury.layer().name());

    tokio::select! {
        () = run_event_loop(mercury, event_rx, effect_tx) => {}
        () = run_effect_loop(effect_rx, emitter, event_tx, title_tx, boot.window_sink, boot.overlay) => {}
    }
    drop(interceptor); // hold the grab until here
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
    // state.handle returns None if the event was unhandled. That is expected! We don't
    // subscribe to only the events that we actually care about in a given state, but instead
    // to all events that we may ever be interested in.
    let effects = state.handle(event).unwrap_or_default();
    info!(event = ?event, effects = ?effects, state = ?state, "dispatch");
    for effect in effects {
        let _ = effect_tx.send(effect);
    }
}

/// The effect loop: read the effect channel and perform each effect, until one of
/// them says to stop.
///
/// Runs on the worker thread, the one consumer of the effect channel, so effects are performed
/// in the order dispatch produced them.
async fn run_effect_loop(
    mut effect_rx: UnboundedReceiver<MercuryEffect>,
    emitter: Emitter,
    event_tx: UnboundedSender<MercuryEvent>,
    title_tx: std::sync::mpsc::Sender<&'static str>,
    windows: Option<WindowSink>,
    overlay: OverlaySink,
) {
    while let Some(effect) = effect_rx.recv().await {
        if perform_effect(
            effect,
            &emitter,
            &event_tx,
            &title_tx,
            windows.as_ref(),
            overlay,
        )
        .is_break()
        {
            break;
        }
    }
}

/// Perform one effect: emit keys, foreground an app, or stop. The effect itself is
/// already on the dispatch record; these are the performance details.
///
/// `Kill` breaks rather than exiting the process, so the way out runs destructors:
/// the `Interceptor` releases the keyboard, the `Stopper` stops main's run loop,
/// and anything registered with the OS gets to deregister.
fn perform_effect(
    effect: MercuryEffect,
    emitter: &Emitter,
    event_tx: &UnboundedSender<MercuryEvent>,
    title_tx: &std::sync::mpsc::Sender<&'static str>,
    windows: Option<&WindowSink>,
    overlay: OverlaySink,
) -> ControlFlow<()> {
    match effect {
        MercuryEffect::Foreground(app) => foreground_app(app),
        MercuryEffect::Tap(Chord { key, flags }) => match emitter.tap(key, flags) {
            Ok(()) => debug!(?key, ?flags, "tapped"),
            Err(e) => warn!(?key, ?flags, error = %e, "tap failed"),
        },
        MercuryEffect::Emit(ke) => match emitter.emit(ke.key, ke.press, ke.flags) {
            Ok(()) => debug!(key = ?ke.key, press = ?ke.press, "emitted"),
            Err(e) => warn!(key = ?ke.key, press = ?ke.press, error = %e, "emit failed"),
        },
        MercuryEffect::SetFrame(target) => set_frame(windows, target),
        MercuryEffect::Copy(what) => copy(what),
        MercuryEffect::Kill => {
            info!("kill: exiting");
            return ControlFlow::Break(());
        }
        MercuryEffect::Timer(timer) => schedule_timer(timer, event_tx),
        MercuryEffect::ShowOverlay(text) => overlay.show(text.to_owned()),
        MercuryEffect::HideOverlay => overlay.hide(),
        // A closed channel means the main thread has gone, which the Kill path handles.
        MercuryEffect::ShowLayer(name) => {
            let _ = title_tx.send(name);
        }
    }
    ControlFlow::Continue(())
}

/// Schedule `timer`: fire its event after its delay, unless the guard the model kept drops first.
///
/// Fire-and-forget on the runtime, like `foreground_app` and `place_window`, so a pending sleep
/// runs off the effect loop.
fn schedule_timer(timer: TimerEffect<MercuryEvent>, event_tx: &UnboundedSender<MercuryEvent>) {
    let TimerEffect {
        delay,
        event,
        cancel: AlwaysEqual(mut cancel),
    } = timer;
    // If the guard dropped before the loop reached this, the receiver is closed and the timer is
    // already cancelled, so there is nothing to spawn.
    if matches!(cancel.try_recv(), Err(TryRecvError::Closed)) {
        return;
    }
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep(delay) => { let _ = event_tx.send(event); }
            _ = cancel => {}
        }
    });
}

/// Set one window's frame, fire-and-forget on its own thread. It takes tens of
/// milliseconds, which is long enough to delay a key the effect loop is about to emit. A
/// detached thread cannot hold up the exit path the way `spawn_blocking` would, which is
/// the same reason `foreground_app` uses one.
fn set_frame(windows: Option<&WindowSink>, target: WindowFrame) {
    let Some(windows) = windows.cloned() else {
        debug!(?target, "no window sink: nothing to place through");
        return;
    };
    std::thread::spawn(move || match windows.set_frame(target) {
        Ok(()) => debug!(?target, "set the window's frame"),
        Err(e) => warn!(?target, error = %e, "set frame failed"),
    });
}

/// Put text on the clipboard, fire-and-forget on its own thread like the rest: `arboard` talks to
/// `NSPasteboard`, and [`Copied::FrontTabUrl`] runs `osascript`, neither of which the effect loop
/// should wait on.
///
/// The pasteboard keeps what it is handed, so the `Clipboard` going out of scope at the end of the
/// thread does not take the text with it.
fn copy(what: Copied) {
    std::thread::spawn(move || {
        let Some(text) = (match what {
            Copied::Text(text) => Some(text),
            Copied::FrontTabUrl(part) => front_tab_url(part),
        }) else {
            return;
        };
        match arboard::Clipboard::new().and_then(|mut board| board.set_text(text.clone())) {
            Ok(()) => debug!(%text, "copied"),
            Err(e) => warn!(%text, error = %e, "copy failed"),
        }
    });
}

/// Ask Chrome for its front tab's URL, and keep `part` of it.
///
/// Only for the tab nobody reported: mercury holds the URL whenever the extension is connected, so
/// this is the fallback, and it is a subprocess and an Apple Events permission rather than a field
/// already in the state. `None` when Chrome answers with nothing usable, which is what a window
/// with no tabs and a denied permission both look like from here.
fn front_tab_url(part: UrlPart) -> Option<String> {
    const SCRIPT: &str =
        "tell application \"Google Chrome\" to get URL of active tab of front window";
    let out = match std::process::Command::new("osascript")
        .args(["-e", SCRIPT])
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            warn!(error = %e, "asking chrome for its front tab failed");
            return None;
        }
    };
    if !out.status.success() {
        warn!(
            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
            "chrome did not answer with a url"
        );
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    match part {
        UrlPart::Whole => (!url.is_empty()).then_some(url),
        UrlPart::Host => host(&url).map(ToOwned::to_owned),
    }
}

/// Foreground an app for real, fire-and-forget on its own thread so the effect
/// loop never blocks on `open`. The watcher reports the app that actually comes
/// up, so nothing here waits on the result (see `app-foregrounding.md`).
fn foreground_app(app: App) {
    let Some(bundle_id) = app.bundle_id() else {
        warn!(app = ?app, "no bundle id; not foregrounding");
        return;
    };
    std::thread::spawn(move || match freddie_app_nav::foreground(bundle_id) {
        Ok(()) => debug!(app = bundle_id, "foregrounded"),
        Err(e) => warn!(app = bundle_id, error = %e, "foreground failed"),
    });
}
