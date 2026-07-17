# a self-clearing overlay in the state tree

An overlay shows for `OVERLAY_DWELL` and clears itself. Its visibility IS its presence in the tree: while an `Overlay::Visible` sits on the root, the overlay is on screen, and clearing it is transitioning to `Overlay::Hidden`, which drops what `Visible` held.

`Visible` holds two RAII guards.

- `TimerGuard` owns the tokio task that fires the clear event after the dwell. Dropping it aborts the task, so a superseded or already-cleared overlay's timer never fires. This is the owned-timer model: the timer lives in the node, and leaving the state drops it.
- `OverlayGuard` is the on-screen overlay. While it exists the overlay is drawn; dropping it hides it. Its presence in the tree is the render's source of truth.

Reshowing clobbers the `Visible`: assigning a fresh one over the old drops the old (aborting its timer, hiding its window) and installs the new, which restarts the dwell. Same content, new timer, so a rapid second show extends the flash.

Abort is not instant. A timer can fire its event at `t = dwell` and be dropped at `t = dwell + ε`; the abort cancels the task but cannot un-send the event already in the channel, and reshows arrive from the keyboard thread while the timer fires from the worker thread, so a superseded timer's fired event can be dispatched after a newer showing is up. Each `TimerGuard` closes over the `OverlayId` it fires; the handler clears only when the live `Visible`'s timer holds that same id, and a superseded showing's timer holds a newer one, so the stale event is discarded. The id is a `u64` generation counter the guard owns, not a field on `Visible` and not an `Arc`/`Weak` token.

```rust
enum Overlay {
    Hidden,
    Visible(TimerGuard, OverlayGuard), // timer drop cancels; screen exists => drawn
}
```

Arming a timer needs the event channel and a runtime, so the root carries a sender the live app injects after construction. Tests construct without one: `show_overlay` then arms an idle timer (no task), and a test drives the clear by queueing the `OverlayTimeout` event itself.

## change 1: the overlay, its guards, and the clear path

The overlay type, the timeout event, and the root handler that clears it. `show_overlay` and `clear_overlay` exist and are exercised by unit tests; no key shows an overlay yet, so a running mercury never leaves `Hidden`.

### the overlay module

New `crates/mercury/src/overlay.rs`:

```rust
//! The transient on-screen overlay: an RAII value whose presence in the tree is its visibility.

use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::{MercuryEvent, OverlayTimeoutEvent};

/// How long a shown overlay stays up before its clear timer fires.
pub(crate) const OVERLAY_DWELL: Duration = Duration::from_millis(1500);

/// Identifies one showing. Each `show_overlay` mints a fresh one and stamps it on the clear
/// timer, so a timer that fires after its showing was superseded carries a stale id.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct OverlayId(u64);

impl OverlayId {
    pub(crate) const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// What the overlay shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OverlayContent(pub &'static str);

/// RAII handle to the pending clear timer. Dropping it aborts the task.
///
/// It closes over the `OverlayId` it fires and keeps it, so the handler can match a fired clear
/// event against the showing still up. `arm` spawns the task and needs a runtime; `idle` is the
/// no-runtime construction (tests, and the live app before its sender is injected).
pub struct TimerGuard {
    id: OverlayId,
    handle: Option<tokio::task::AbortHandle>,
}

impl TimerGuard {
    /// Spawn a task that sleeps `OVERLAY_DWELL`, then fires the clear event for `id`. The guard
    /// aborts it on drop.
    pub(crate) fn arm(tx: &UnboundedSender<MercuryEvent>, id: OverlayId) -> Self {
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(OVERLAY_DWELL).await;
            let _ = tx.send(MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id }));
        })
        .abort_handle();
        Self {
            id,
            handle: Some(handle),
        }
    }

    /// No task: the timer never fires, so the overlay stays until something else clears it.
    pub(crate) const fn idle(id: OverlayId) -> Self {
        Self { id, handle: None }
    }

    /// The id this timer fires. The handler matches a clear event against it.
    pub(crate) const fn id(&self) -> OverlayId {
        self.id
    }
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

impl std::fmt::Debug for TimerGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TimerGuard({:?}, {})",
            self.id,
            if self.handle.is_some() { "armed" } else { "idle" }
        )
    }
}

/// The on-screen overlay. Holding it is drawing it; the renderer reads `content` off the active
/// `Overlay::Visible`.
///
/// The window teardown lands with `freddie_overlay`; until then the guard is the model's record
/// that something is on screen, and a `Drop` that hides the window slots in here.
#[derive(Debug)]
pub struct OverlayGuard {
    content: OverlayContent,
}

impl OverlayGuard {
    pub(crate) const fn show(content: OverlayContent) -> Self {
        Self { content }
    }

    #[must_use]
    pub const fn content(&self) -> OverlayContent {
        self.content
    }
}

/// The overlay, present or not. `Visible` while on screen; dropping it (the transition to
/// `Hidden`, or a reshow overwriting it) aborts its timer and hides it.
#[derive(Debug, Default)]
pub enum Overlay {
    #[default]
    Hidden,
    Visible(TimerGuard, OverlayGuard),
}

impl Overlay {
    /// The overlay on screen, or `None` when hidden. For the renderer.
    #[must_use]
    pub const fn visible(&self) -> Option<&OverlayGuard> {
        match self {
            Self::Hidden => None,
            Self::Visible(_, screen) => Some(screen),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No runtime here, so timers are idle; the id logic is what these check.

    fn shown(overlay: &Overlay) -> Option<OverlayId> {
        match overlay {
            Overlay::Hidden => None,
            Overlay::Visible(timer, _) => Some(timer.id()),
        }
    }

    #[test]
    fn show_then_clear() {
        let mut m = crate::Mercury::default();
        m.show_overlay(OverlayContent("a"));
        assert_eq!(shown(m.overlay()), Some(OverlayId(0)));
        m.clear_overlay(OverlayId(0));
        assert!(matches!(m.overlay(), Overlay::Hidden));
    }

    #[test]
    fn reshow_supersedes_and_a_stale_timer_is_ignored() {
        let mut m = crate::Mercury::default();
        m.show_overlay(OverlayContent("a"));
        m.show_overlay(OverlayContent("b")); // clobber: new id
        assert_eq!(shown(m.overlay()), Some(OverlayId(1)));
        m.clear_overlay(OverlayId(0)); // the superseded timer's event: stale, ignored
        assert_eq!(shown(m.overlay()), Some(OverlayId(1)));
        m.clear_overlay(OverlayId(1)); // the live timer's event
        assert!(matches!(m.overlay(), Overlay::Hidden));
    }
}
```

### the timeout event and trigger

`crates/mercury/src/sources.rs`, appended:

```rust
use crate::OverlayId;

/// A trigger matching the overlay's clear timer firing.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct OverlayTimeout;

/// A fired overlay-clear timer, carrying the id it was armed with so the handler can tell the
/// live timer from a superseded one.
#[derive(Debug)]
pub struct OverlayTimeoutEvent {
    pub id: OverlayId,
}

impl EventTrigger for OverlayTimeout {
    type Event = OverlayTimeoutEvent;
    fn is_matching(&self, _ev: &OverlayTimeoutEvent) -> bool {
        true
    }
}
```

### the unified event and trigger

`crates/mercury/src/model.rs`, before:

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyModifierKey(AnyModifierKey),
    AnyNonModifierKey(AnyNonModifierKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
}
```

after (add the variant, and the matching `use`):

```rust
pub enum MercuryTrigger {
    Key(Key),
    KeyPress(KeyPress),
    AnyModifierKey(AnyModifierKey),
    AnyNonModifierKey(AnyNonModifierKey),
    Foregrounded(Foregrounded),
    Quit(Quit),
    OverlayTimeout(OverlayTimeout),
}
```

`MercuryEvent`, before:

```rust
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
}
```

after:

```rust
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(QuitEvent),
    OverlayTimeout(OverlayTimeoutEvent),
}
```

`#[try_into(ref)]` gains `TryFrom<&MercuryEvent> for &OverlayTimeoutEvent`, the narrow dispatch uses. `model.rs`'s `use crate::{..}` picks up `OverlayTimeout` and `OverlayTimeoutEvent`.

### the root fields and methods

`crates/mercury/src/state.rs`, `Mercury`, before:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub held: HeldModifiers,
    #[resolve_into]
    layer: Layer,
}
```

after:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub held: HeldModifiers,
    /// The transient overlay. Root state: shown from any layer, and it outlives layer changes,
    /// so it is not a node. `Hidden` unless a `show_overlay` put it up.
    overlay: Overlay,
    /// Mints the id each showing stamps on its clear timer.
    next_overlay_id: OverlayId,
    /// The channel the overlay timer fires its clear event back through. `None` until the live
    /// app injects it (tests never do); without it `show_overlay` arms an idle timer.
    event_tx: Option<UnboundedSender<MercuryEvent>>,
    #[resolve_into]
    layer: Layer,
}
```

Its `#[bind(..)]`, before:

```rust
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
```

after:

```rust
#[bind(
    Foregrounded => on_foregrounded,
    Quit => on_quit,
    OverlayTimeout => on_overlay_timeout,
    AnyModifierKey => on_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
```

The `Default` impl, before:

```rust
impl Default for Mercury {
    fn default() -> Self {
        Self {
            foreground: Foreground::default(),
            held: HeldModifiers::default(),
            layer: Layer::Typing(TypingLayer {}),
        }
    }
}
```

after:

```rust
impl Default for Mercury {
    fn default() -> Self {
        Self {
            foreground: Foreground::default(),
            held: HeldModifiers::default(),
            overlay: Overlay::Hidden,
            next_overlay_id: OverlayId::default(),
            event_tx: None,
            layer: Layer::Typing(TypingLayer {}),
        }
    }
}
```

New methods on `impl Mercury`:

```rust
/// Show `content`, or extend an overlay already up, for [`OVERLAY_DWELL`]. Mints a fresh id and
/// arms a clear timer carrying it; the assignment drops the previous `Visible`, aborting its
/// timer and hiding it. A fresh id means a previous timer that already fired (the abort race)
/// carries a stale id and is ignored when it arrives.
pub fn show_overlay(&mut self, content: OverlayContent) {
    let id = self.next_overlay_id;
    self.next_overlay_id = id.next();
    let timer = match &self.event_tx {
        Some(tx) => TimerGuard::arm(tx, id),
        None => TimerGuard::idle(id),
    };
    self.overlay = Overlay::Visible(timer, OverlayGuard::show(content));
}

/// The overlay's clear timer fired. Hide it only if the `Visible` still up was armed with `id`; a
/// superseded overlay's timer holds a newer id, so the stale event is discarded here.
pub fn clear_overlay(&mut self, id: OverlayId) {
    if matches!(&self.overlay, Overlay::Visible(timer, _) if timer.id() == id) {
        self.overlay = Overlay::Hidden;
    }
}

/// The overlay, for the renderer.
#[must_use]
pub const fn overlay(&self) -> &Overlay {
    &self.overlay
}

/// Inject the sender the overlay timer fires through. The live app calls this once after
/// construction; tests leave it unset.
pub fn set_event_tx(&mut self, event_tx: UnboundedSender<MercuryEvent>) {
    self.event_tx = Some(event_tx);
}
```

`state.rs` gains `use tokio::sync::mpsc::UnboundedSender;` and pulls `Overlay`, `OverlayContent`, `OverlayGuard`, `OverlayId`, `TimerGuard` from the crate. `OVERLAY_DWELL` and `OverlayTimeoutEvent` stay inside `overlay.rs`, where `TimerGuard::arm` uses them.

### the handler

New `crates/mercury/src/handlers/overlay.rs`:

```rust
//! The overlay's clear handler.

use bind::Node;

use crate::state::Mercury;
use crate::{MercuryEffect, OverlayTimeoutEvent};

/// The overlay's clear timer fired: hide it if this timer is still the live one. Bound at the
/// root, so it fires whatever layer is active.
pub(crate) fn on_overlay_timeout(
    ev: &OverlayTimeoutEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.clear_overlay(ev.id);
    Vec::new()
}
```

`crates/mercury/src/handlers/mod.rs`, before:

```rust
mod app;
mod foreground;
mod home;
mod nav;
mod quit;
mod resize;
mod root;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use root::*;
pub(crate) use typing::*;
```

after (add the module and its glob):

```rust
mod app;
mod foreground;
mod home;
mod nav;
mod overlay;
mod quit;
mod resize;
mod root;
mod typing;

pub(crate) use app::*;
pub(crate) use foreground::*;
pub(crate) use home::*;
pub(crate) use nav::*;
pub(crate) use overlay::*;
pub(crate) use quit::*;
pub(crate) use resize::*;
pub(crate) use root::*;
pub(crate) use typing::*;
```

### the exports

`crates/mercury/src/lib.rs`, before:

```rust
mod effect;
mod handlers;
mod model;
mod sources;
mod state;

pub use effect::{MercuryEffect, Placement};
pub use model::{MercuryEvent, MercuryStruct, MercuryTrigger};
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, Quit, QuitEvent,
};
```

after:

```rust
mod effect;
mod handlers;
mod model;
mod overlay;
mod sources;
mod state;

pub use effect::{MercuryEffect, Placement};
pub use model::{MercuryEvent, MercuryStruct, MercuryTrigger};
pub use overlay::{Overlay, OverlayContent, OverlayGuard, OverlayId, TimerGuard};
pub use sources::{
    AnyModifierKey, AnyNonModifierKey, App, ForegroundEvent, Foregrounded, OverlayTimeout,
    OverlayTimeoutEvent, Quit, QuitEvent,
};
```

`OVERLAY_DWELL` stays `pub(crate)`: it is the internal dwell, not part of the API.

At this point mercury compiles, the unit tests pass, and dispatching an `OverlayTimeout` event clears an overlay. Nothing shows one yet.

## change 2: a key shows the overlay, and the live app arms real timers

`o` in home flashes the foregrounded app's name; repeat it to extend. The live app injects its event sender so the armed timer actually fires.

### the app name

`crates/mercury/src/sources.rs`, appended to `impl App`:

```rust
/// The app's overlay label.
#[must_use]
pub const fn name(self) -> &'static str {
    match self {
        Self::Chrome => "Chrome",
        Self::Finder => "Finder",
        Self::Ghostty => "Ghostty",
        Self::Zed => "Zed",
        Self::Other => "Other",
    }
}
```

### the show handler and binding

`crates/mercury/src/handlers/home.rs`, appended:

```rust
use crate::OverlayContent;

/// `o` in home: flash the foregrounded app's name. Pressing it again extends the flash.
pub(crate) fn show_app_overlay(
    _ev: &KeyEvent,
    node: Node<HomeLayerPath, ()>,
) -> Vec<MercuryEffect> {
    let root = node.parent.ascend_to::<MercuryPath>();
    let content = OverlayContent(root.foreground.app().name());
    root.show_overlay(content);
    Vec::new()
}
```

`crates/mercury/src/state.rs`, `HomeLayer`'s `#[bind(..)]`, before:

```rust
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer {}
```

after:

```rust
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyO.down() => show_app_overlay,
    Key::KeyQ.down() => quit,
)]
pub struct HomeLayer {}
```

### the live app injects its sender

`crates/mercury/src/main.rs`, `run`, before:

```rust
    let mut mercury = Mercury::default();
    mercury.foreground.on_foregrounded_app_event(
        freddie_app_nav::frontmost()
            .map_or(App::Other, |bundle_id| App::from_bundle_id(&bundle_id)),
    );
```

after (hand mercury the sender its overlay timers fire through):

```rust
    let mut mercury = Mercury::default();
    mercury.set_event_tx(event_tx.clone());
    mercury.foreground.on_foregrounded_app_event(
        freddie_app_nav::frontmost()
            .map_or(App::Other, |bundle_id| App::from_bundle_id(&bundle_id)),
    );
```

The timer arms on the worker's current-thread runtime (the event loop is `block_on`ed there), sleeps, and sends `OverlayTimeout` back onto `event_tx`, where it dispatches like any event. No effect and no change to the effect loop: the overlay lives entirely in the event-and-state world, and the tokio task is the only asynchrony, owned by the `TimerGuard`.

### tests

`crates/mercury/tests/transitions.rs`, new cases driving through `SimpleRunner` (no runtime, so the timer is idle and the `OverlayTimeout` event is queued by hand to stand in for it):

- `o` in home shows the overlay carrying the frontmost app's name, and dispatch returns no effect.
- A second `o` before the clear keeps the overlay up under a newer id (extend).
- An `OverlayTimeout` for the superseded id is a no-op; one for the live id hides it.
- `o` outside home does nothing (home binds it; other layers do not).

The existing transition tests are untouched: `show_overlay` returns no effect and mutates only `overlay`/`next_overlay_id`, which they do not assert.

## open questions

- `OverlayGuard`'s `Drop` is the seam for tearing down the on-screen window; the window itself is `freddie_overlay`, unbuilt. Until it exists the guard only records visibility, and nothing draws.
- The show trigger here is one demonstration key. Flashing on layer entry (voicemode's `showBrief(layer)`) is the motivating case and reuses `show_overlay` unchanged; it is left out because it is a product choice, not a mechanism.
- `set_event_tx` puts a channel on the root, which is what lets a handler arm a timer without threading a context through `bind`'s dispatch. The alternative is a handler context parameter, a cross-crate change to `bind`; nothing else needs it yet.
- Whether other overlays (not app names) want richer `OverlayContent` than a `&'static str`.
