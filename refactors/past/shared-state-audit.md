# bringing the shared-state sites in line with the standard

`CLAUDE.md`'s "Shared state and interior mutability" section says `Arc`, `Rc`, `Mutex`, `RwLock`, `Cell`, `RefCell`, `OnceLock`, `lazy_static`, `thread_local!`, and atomics are almost always the wrong reach: the model is a single-threaded pure function of state and event, and the preferred way to move data between threads is a channel whose sender is freely `Send` while the receiver stays pinned to one thread.

Seven files predate that rule. This doc records the disposition of each: the ones that fail the standard are changed, the ones a constraint genuinely forces are kept with a note so a later audit does not re-open them. Two of the changes are large enough to be their own docs.

Full survey of the tree: the ten other crates (`bind`, `bind_macro`, `derive_support`, `freddie_keys`, `freddie_keyboard`, `freddie_menu_bar`, `freddie_single_instance`, `freddie_app_nav`, `laserbeam`, `mercury`) use none of these primitives.

## Change 1: `freddie_main_loop`'s stop signal becomes a channel

Landed in `0f97992`. The `Arc<AtomicBool>` is a `std::sync::mpsc` channel now: `Stopper` holds the `Sender` and sends before `CFRunLoop::stop` in its `Drop`, `MainLoop` holds the `Receiver` and keeps turning while `try_recv` is `Empty`. The `main_loop() -> (MainLoop, Stopper)` signature is unchanged, so `daemon.rs` was untouched.

## Change 2: `freddie/src/timer.rs`'s id source moves onto the root

Moved to `refactors/pending/timer-ids-on-root.md`. The `static AtomicU64` becomes a plain `TimerIds(u64)` field on the root, minted with `&mut`, which makes the id a function of state and lets `TimerFired` drop its `testing`-only equality hack. The same change removes the per-call closure through a `From<TimerFired>` bound.

## Change 3: `freddie_overlay`'s marshaling to the main thread

Moved to `refactors/pending/overlay-marshaling.md`. The GCD dispatch into a `thread_local` panel table becomes a channel drained on `MainLoop::run`'s `on_wake`, which deletes the `thread_local!`, the `OverlayId`, and the table. It replaces the sink's whole delivery path, so it is a design pass of its own rather than a mechanical edit.

## Change 4: `freddie_event_socket`'s handler funnels through one task

Landed in `821c3cc`. `Arc<F>` is gone: every connection forwards its text frames over an `mpsc` whose `Sender` is `Send` and `Clone`, and one task owns `on_message` and drains them. Serializing the calls onto one task costs nothing (the callback is already required non-blocking) and drops the `Sync` bound the shared `Fn` needed.

## Reviewed and kept

These sites hold a primitive because a platform or trait constraint forces it. Each gets one line added to the type or function's doc comment recording that the constraint was reviewed against the standard and stands, so the next audit does not re-litigate.

- `crates/freddie_windows/src/lib.rs:142` `Elements(Mutex<HashMap<WindowId, Arc<Element>>>)`, the one genuinely cross-thread case. The strong `Arc<Elements>` is on `WatcherState` (`:556`), held by the main thread; `WindowSink` (`:153`) holds a `Weak<Elements>` and upgrades it to `Arc` per call (`:985`), so the `Mutex` is the thing that actually crosses threads. `WindowSink::set_frame` runs off the main thread by design (tens of milliseconds per placement, which the run loop cannot spend) while the main thread mutates the table as windows open and close. A channel to the main thread would put those milliseconds back on the run loop, which is the thing the design avoids. The `Weak` is deliberate: the watcher holds the only strong reference, so dropping the watcher ends every sink's access.
- `crates/freddie_windows/src/lib.rs:976,559` `state: Rc<WatcherState>` and `apps: RefCell<HashMap<Pid, AppObserver>>`. The C-callback-reaches-state pattern from `docs/platform-apis.md`: `AXObserver` callbacks reach state through a `refcon`, and the `'static` launch and terminate closures cannot borrow the `Watcher`. Single-threaded shared ownership with C callbacks.
- `crates/freddie_cli/src/logging.rs:86` `thread_local! { static STAMPED: RefCell<Vec<u8>> }`. `io::Write::write` is a synchronous trait method `tracing` calls on whatever thread logs, so there is no owning thread to send to. The buffer is per-thread scratch reused to avoid a per-record allocation on the daemon's hot path.
- `crates/freddie_cli/src/logging.rs:94` `static STAMP: OnceLock<String>`. The process pid string, built once and immutable. This one could instead be computed at `init` and owned inside `WithPid<W>`, removing the `OnceLock`; fold that in when `logging.rs` is next touched, but it is not worth a standalone change.
- `crates/freddie_event_socket/tests/socket.rs:19` `Arc<Mutex<Vec<String>>>`. A test recorder. `CLAUDE.md` scopes the model standard to the model, and a test that collects what a callback saw is outside it.
