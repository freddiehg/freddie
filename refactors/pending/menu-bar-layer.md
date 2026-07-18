# the active layer, in the menu bar

The status item shows a fixed glyph, so nothing on screen says which layer is active. You find out by pressing a key and seeing what happens. The item shows the layer's name beside the glyph instead, changing as the layer does.

## Where the name comes from, and how it gets there

The layer changes in exactly one place, `Mercury::set_layer`, which already returns the effects a transition implies. So the name rides out as one more effect, and the model stays a pure function of state and event.

Getting it to the status item is the whole of the problem. `TrayIcon::set_title` goes through a `RefCell`, and an `NSStatusItem` may only be touched on the main thread, so `MenuBar` is `!Send` and lives on the main thread that built it. The effect loop runs on the worker.

The main thread is already parked in `MainLoop::run`, waking at least every `SLICE` (100ms) to check whether it has been stopped, and more often whenever an `AppKit` event arrives. So it takes a callback and drains a channel of pending titles on each wake, which needs no main-queue dispatch, no `unsafe`, and no extra wakeups: the wake already happens.

The cost is latency. A layer change can take up to 100ms to show. That is imperceptible for a passive indicator, and it is the reason to prefer this over `dispatch2::DispatchQueue::main().exec_async`, which would apply it immediately but needs the `TrayIcon` reachable from a `Send + 'static` closure — a thread-local or a main-thread-only static, since neither `TrayIcon` nor a reference to it can cross threads.

## change 1: the layer knows its name

`crates/mercury/src/state/mod.rs`, on `impl Layer`:

```rust
    /// What the status item calls this layer.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Home(_) => "Home",
            Self::Nav(_) => "Nav",
            Self::Resize(_) => "Resize",
            Self::Typing(_) => "Typing",
            Self::InApp(_) => "App",
        }
    }
```

A test in `crates/mercury/tests/transitions.rs` asserts each layer's name, so a new layer forces a name rather than defaulting to something.

## change 2: a transition says what to show

`crates/mercury/src/effect.rs`, a new variant on `MercuryEffect`:

```rust
    /// Show this layer's name in the menu bar. Produced only by `set_layer`, so the item and the
    /// model cannot disagree about which layer is active.
    ShowLayer(&'static str),
```

`crates/mercury/src/state/mod.rs`, `set_layer`, before:

```rust
        self.layer = into;
        self.typing_state.jk = KeySequence::new(JK, Some(JK_TIMEOUT));
        match (before_passthrough, after_passthrough) {
            (true, false) => self.typing_state.held.close(),
            (false, true) => self.typing_state.held.open(),
            _ => Vec::new(),
        }
```

after:

```rust
        self.layer = into;
        self.typing_state.jk = KeySequence::new(JK, Some(JK_TIMEOUT));
        let mut effects = match (before_passthrough, after_passthrough) {
            (true, false) => self.typing_state.held.close(),
            (false, true) => self.typing_state.held.open(),
            _ => Vec::new(),
        };
        effects.push(MercuryEffect::ShowLayer(self.layer.name()));
        effects
```

Every existing test that asserts a transition's effects gains the `ShowLayer` at the end of the vector. There are enough of them that a helper is worth it in `transitions.rs`:

```rust
// A transition also tells the menu bar which layer it landed in.
fn shows(layer: &'static str) -> MercuryEffect {
    MercuryEffect::ShowLayer(layer)
}
```

The initial layer needs one too, since nothing has transitioned yet when the process starts: `main` sends `Mercury::default().layer().name()` before the loops begin, or the effect loop is primed with a `ShowLayer` for it.

## change 3: the status item takes a title

`crates/freddie_menu_bar/src/lib.rs`. `MenuBar` keeps the `TrayIcon` rather than dropping it into `_tray`, since setting a title needs it:

before:

```rust
pub struct MenuBar {
    _tray: TrayIcon,
}
```

after:

```rust
pub struct MenuBar {
    tray: TrayIcon,
}

impl MenuBar {
    /// Set the text shown beside the glyph, or clear it with `None`.
    ///
    /// Main thread only, like everything else about a status item: `TrayIcon` is `!Send` and
    /// holding one is what keeps this method reachable only from the thread that built it.
    pub fn set_title(&self, title: Option<&str>) {
        self.tray.set_title(title);
    }
}
```

## change 4: the main loop applies pending titles

`crates/freddie_main_loop/src/lib.rs`. `run` takes a callback it calls on every wake, so a caller can do main-thread-only work without owning the loop:

before:

```rust
    pub fn run(self) {
```

after:

```rust
    /// `on_wake` runs on each pass, which is at least every `SLICE` and again whenever an
    /// `AppKit` event arrives. It runs ON the main thread, so it is where a caller does
    /// main-thread-only work that came from elsewhere; it must return promptly, for the same
    /// reason a source callback must.
    pub fn run(self, mut on_wake: impl FnMut()) {
```

with `on_wake()` called once per iteration of the `while !self.stop.load(..)` loop, after the event is dispatched. The crate-level doc example becomes `main_loop.run(|| {})`.

## change 5: the wiring

`crates/mercury/src/main.rs`. A channel carries names from the effect loop to the main thread, which is the only thread that may touch the item:

```rust
// Titles for the status item. The effect loop (worker) sends; the main thread applies them on
// its next wake, since an NSStatusItem is main-thread-only. A std channel, not tokio's: the
// receiving end is the main thread, which is not in the runtime.
let (title_tx, title_rx) = std::sync::mpsc::channel::<&'static str>();
```

`title_tx` is cloned into `run` and reaches `perform_effect`, which gains an arm:

```rust
        MercuryEffect::ShowLayer(name) => {
            // A closed channel means main has gone, which the Kill path handles.
            let _ = title_tx.send(name);
        }
```

and `main_loop.run` drains it, applying only the last one, since intermediate layers in one batch are not worth drawing:

```rust
    main_loop.run(|| {
        if let Some(name) = title_rx.try_iter().last() {
            menu_bar.set_title(Some(name));
        }
    });
```

`menu_bar` moves into the closure rather than being dropped after `run` returns; the icon comes down when the closure does, which is when `run` returns, so the lifetime is unchanged.

## change 6: verification

The model half is testable and tested by change 2. The rest is not: it is a status item on a real menu bar. Verify by running mercury and watching the item change as you move between layers, including that it reads `Typing` at startup with no key pressed.
