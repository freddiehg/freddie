# a self-clearing overlay, shown by effects

The overlay is an external AppKit window, not model state. The model cannot own an AppKit window by RAII across threads (the state tree lives on the worker thread, AppKit is main-thread-only), so the overlay is controlled entirely by effects: `ShowOverlay(text)` puts it up, `HideOverlay` takes it down. There is no overlay content or visibility in the tree.

`o` in any non-typing layer shows that layer's keymap hint and arms a hide timer. The overlay hides when you change layers, or after the dwell. One binding each side:

- `o => show_overlay` in every non-typing layer. `o` is a keystroke, so binding it at the root would fire in typing too and you could never type the letter, which is why it is per-layer and typing passes `o` through.
- `OverlayTimeout => hide_overlay` once at the root. The timeout is a timer event, not a keystroke, and hiding is not opt-in, so a single root binding covers every layer.

The overlay's trace in the model is one field on the root, `overlay: Option<ShownOverlay>`, holding the showing's generation id and the guard for its pending hide timer. It is a single thing shown from any layer, so its state is a root concern, not a per-layer one (unlike the return-home timer, which each layer owns independently). Alongside it, `next_overlay_id` mints the id. The three paths:

- `show_overlay` mints an id, stamps it on the hide timer's event, and stores the id and guard. Reassigning drops any previous guard, cancelling a still-pending timer, so a rapid second `o` supersedes.
- `hide_overlay(id)` hides only if the showing still up was armed with `id`, then clears the field.
- `set_layer` takes the field: if one was up, push `HideOverlay`. Taking it drops the guard, cancelling the timer.

The generation id is what makes the hide safe. Dropping the guard cancels the timer only if its sleep has not completed; a timer that fired in the moment before the drop has already put its `OverlayTimeout` on the event channel, and that cannot be un-sent. Since `HideOverlay` hides the one external panel, a stale timeout from a superseded showing would hide the live overlay, leaving nothing up. So the timeout carries the id it was armed with, and `hide_overlay` ignores one whose id is not the showing still up. `set_layer`'s hide needs no id: it is synchronous, you are leaving, so it hides whatever is up.

The `DropGuard` is the drop-signals-a-receiver half of today's `TimerGuard`, pulled out of `freddie::timer` (change 1) so the timer and the overlay share it. It is a pure RAII primitive: the effect that carries the paired receiver wraps it in `AlwaysEqual` for its `testing` equality; the guard itself carries no such concern.

## change 1: extract `DropGuard` in freddie

The cancel mechanism, a sender whose drop wakes a paired receiver, is today buried in `TimerGuard`. Pull it into its own `freddie` primitive so the overlay's hide timer reuses it. `TimerGuard` goes away; the effect keeps the receiver and the `AlwaysEqual` around it.

### the new module

New `crates/freddie/src/drop_guard.rs`:

```rust
//! A guard whose drop cancels a paired async job.

use tokio::sync::oneshot;

/// The cancelling half of a drop pair. The owning node holds it; dropping it (a transition that
/// replaces the node, or a clobber that overwrites it) closes the channel and wakes the paired
/// receiver, so whatever waits on that receiver tears down at once.
///
/// A pure RAII primitive: it knows nothing about testing equality. A consumer that needs a
/// comparable effect wraps its own half in `AlwaysEqual` (see [`TimerEffect`](crate::TimerEffect)).
#[must_use = "dropping the guard cancels immediately"]
pub struct DropGuard(
    // Held only to be dropped: dropping the sender wakes the paired receiver. Never read.
    #[allow(dead_code)] oneshot::Sender<()>,
);

impl std::fmt::Debug for DropGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DropGuard")
    }
}

/// Build a linked guard/receiver pair. The guard goes in the node; the receiver rides an effect to
/// whatever performs the cancellable job.
#[must_use]
pub fn drop_guard() -> (DropGuard, oneshot::Receiver<()>) {
    let (sender, receiver) = oneshot::channel();
    (DropGuard(sender), receiver)
}
```

### the timer, on top of it

`crates/freddie/src/timer.rs`, before:

```rust
use std::time::Duration;

use tokio::sync::oneshot;

use crate::AlwaysEqual;

/// The cancelling half, held by the node that owns the timer.
///
/// Dropping it (a transition that replaces the node, or a clobber that overwrites the guard)
/// cancels the timer at once, because the paired receiver wakes when this sender goes.
#[must_use = "dropping the guard cancels the timer immediately"]
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerGuard(
    // Held for its `Drop`: dropping the sender wakes the paired receiver and cancels the timer. It
    // is read only under `testing` (the equality derive), so it is otherwise never read.
    #[cfg_attr(not(feature = "testing"), allow(dead_code))] AlwaysEqual<oneshot::Sender<()>>,
);

/// The scheduling half: a delay, the event to fire, and the cancel channel.
///
/// A handler returns it as an effect and the effect loop pattern-matches it to schedule. It owns
/// the event and the receiver, so it is used once. The receiver sits in `AlwaysEqual`, so the
/// effect's `testing` equality is the delay and the event.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerEffect<E> {
    pub delay: Duration,
    pub event: E,
    pub cancel: AlwaysEqual<oneshot::Receiver<()>>,
}

/// Build a linked guard/event pair that fires `event` after `delay`.
pub fn timer_effect_and_guard<E>(delay: Duration, event: E) -> (TimerGuard, TimerEffect<E>) {
    let (sender, cancel) = oneshot::channel();
    (
        TimerGuard(AlwaysEqual(sender)),
        TimerEffect {
            delay,
            event,
            cancel: AlwaysEqual(cancel),
        },
    )
}
```

after (the guard comes from `drop_guard`; the effect wraps its own receiver in `AlwaysEqual`):

```rust
use std::time::Duration;

use tokio::sync::oneshot;

use crate::AlwaysEqual;
use crate::drop_guard::{DropGuard, drop_guard};

/// The scheduling half: a delay, the event to fire, and the cancel channel.
///
/// A handler returns it as an effect and the effect loop pattern-matches it to schedule. It owns
/// the event and the receiver, so it is used once. The receiver sits in `AlwaysEqual`, so the
/// effect's `testing` equality is the delay and the event; the effect, not the guard, carries the
/// testing concern.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TimerEffect<E> {
    pub delay: Duration,
    pub event: E,
    pub cancel: AlwaysEqual<oneshot::Receiver<()>>,
}

/// Build a linked guard/event pair that fires `event` after `delay`. The guard cancels the timer
/// on drop.
pub fn timer_effect_and_guard<E>(delay: Duration, event: E) -> (DropGuard, TimerEffect<E>) {
    let (guard, receiver) = drop_guard();
    (
        guard,
        TimerEffect {
            delay,
            event,
            cancel: AlwaysEqual(receiver),
        },
    )
}
```

### the exports

`crates/freddie/src/lib.rs`, before:

```rust
//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;
pub mod timer;

pub use always_equal::AlwaysEqual;
pub use timer::{TimerEffect, TimerGuard, timer_effect_and_guard};
```

after:

```rust
//! freddie: a framework for typed event-to-state machines. Work in progress.

pub mod always_equal;
pub mod drop_guard;
pub mod timer;

pub use always_equal::AlwaysEqual;
pub use drop_guard::{DropGuard, drop_guard};
pub use timer::{TimerEffect, timer_effect_and_guard};
```

### the mercury side of the rename

The return-home guard the layers hold is now a `DropGuard`. In `crates/mercury/src/state/nav.rs`, `resize.rs`, and `app.rs`, the field and its `use` change identically. `nav.rs`, before:

```rust
use freddie::TimerGuard;
// ...
pub struct NavLayer {
    // Held for its `Drop`: dropping the guard cancels nav's return-home timer.
    #[allow(dead_code)]
    timeout: TimerGuard,
}
```

after:

```rust
use freddie::DropGuard;
// ...
pub struct NavLayer {
    // Held for its `Drop`: dropping the guard cancels nav's return-home timer.
    #[allow(dead_code)]
    timeout: DropGuard,
}
```

`crates/mercury/src/state/mod.rs`, before:

```rust
use freddie::{TimerGuard, timer_effect_and_guard};
// ...
fn arm_return_home() -> (TimerGuard, MercuryEffect) {
```

after:

```rust
use freddie::{DropGuard, timer_effect_and_guard};
// ...
fn arm_return_home() -> (DropGuard, MercuryEffect) {
```

`crates/mercury/tests/transitions.rs` destructures the pair as `let (_guard, effect) = ...`, so it needs no change.

## change 2: the overlay effects, content, and bindings

`o` shows the active layer's hint and arms the hide timer; `OverlayTimeout` and every layer change hide it. Rendering is a `debug!` line here; change 3 makes it draw.

### the effects

`crates/mercury/src/effect.rs`, `MercuryEffect`, add the two overlay effects after `Timer`:

```rust
    /// Arm a timer. The effect loop schedules it; it fires its event after the delay unless the
    /// guard held by the state that asked for it drops first.
    Timer(TimerEffect<MercuryEvent>),
    /// Show the overlay with this text, replacing whatever it shows now.
    ShowOverlay(&'static str),
    /// Hide the overlay. A no-op if nothing is up.
    HideOverlay,
```

### the timeout trigger, its event, and the id

`crates/mercury/src/sources.rs`, appended. The timeout carries the generation id, so it is a manual `EventTrigger`, not a `self_trigger!` like `LayerTimeout`:

```rust
/// Identifies one showing. Each `show_overlay` mints a fresh one and stamps it on the hide timer,
/// so a timer that fires after its showing was superseded carries a stale id.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct OverlayId(pub u64);

impl OverlayId {
    pub(crate) const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// A fired overlay hide timer, carrying the id it was armed with so the handler can tell the live
/// showing from a superseded one.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct OverlayTimeoutEvent {
    pub id: OverlayId,
}

/// The overlay hide timer's trigger. It matches any fired hide event; the id it carries is compared
/// in the handler, not here.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct OverlayTimeout;

impl EventTrigger for OverlayTimeout {
    type Event = OverlayTimeoutEvent;
    fn is_matching(&self, _ev: &OverlayTimeoutEvent) -> bool {
        true
    }
}
```

### the unified event and trigger

`crates/mercury/src/model.rs`, the `use`, before:

```rust
use crate::{
    AnyModifierKey, AnyNonModifierKey, ForegroundEvent, Foregrounded, LayerTimeout, MercuryEffect,
    Quit,
};
```

after:

```rust
use crate::{
    AnyModifierKey, AnyNonModifierKey, ForegroundEvent, Foregrounded, LayerTimeout, MercuryEffect,
    OverlayTimeout, OverlayTimeoutEvent, Quit,
};
```

`MercuryTrigger` gains `OverlayTimeout(OverlayTimeout)` and `MercuryEvent` gains `OverlayTimeout(OverlayTimeoutEvent)`. `MercuryEvent`, after:

```rust
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug, derive_more::TryInto)]
#[try_into(ref)]
pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Quit(Quit),
    LayerTimeout(LayerTimeout),
    OverlayTimeout(OverlayTimeoutEvent),
}
```

### the layer's content

`crates/mercury/src/state/mod.rs`. Add the dwell next to the return-home timeout:

```rust
/// How long the overlay stays up before its hide timer fires.
pub const OVERLAY_DWELL: Duration = Duration::from_secs(10);
```

Add the content hint to `impl Layer`, beside `is_passthrough`:

```rust
/// The keymap hint the overlay shows for this layer, read when `o` shows it. Typing never binds
/// `o`, so its hint is unreachable.
#[must_use]
pub const fn get_overlay_content(&self) -> &'static str {
    match self {
        Self::Home(_) => "n nav  r resize  t typing  i in-app  q quit",
        Self::Nav(_) => "c chrome  f finder  g ghostty  z zed",
        Self::Resize(_) => "arrows place the window",
        Self::InApp(_) => "n nav  t typing",
        Self::Typing(_) => "typing",
    }
}
```

### the root state

The showing up, if any, is one field. Add the struct near `Mercury`:

```rust
/// The overlay on screen: its generation id, and the guard whose drop cancels the pending hide.
#[derive(Debug)]
struct ShownOverlay {
    id: OverlayId,
    // Held for its `Drop`: dropping the guard cancels the pending hide timer.
    #[allow(dead_code)]
    timer: DropGuard,
}
```

The `Mercury` struct, before:

```rust
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    /// The physical truth about which modifier keys are down [..]. See [`HeldModifiers`].
    pub held: HeldModifiers,
    /// The active layer. Private, and written only through [`set_layer`](Mercury::set_layer) [..].
    #[resolve_into]
    layer: Layer,
}
```

after:

```rust
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    /// The physical truth about which modifier keys are down [..]. See [`HeldModifiers`].
    pub held: HeldModifiers,
    /// The overlay currently up, if any. The overlay is an external window driven by effects; this
    /// is its only trace in the model, held at the root because there is one overlay across all
    /// layers.
    overlay: Option<ShownOverlay>,
    /// Mints the generation id each showing stamps on its hide timer, so a stale timeout (one whose
    /// showing was superseded) is ignored rather than hiding the live overlay.
    next_overlay_id: OverlayId,
    /// The active layer. Private, and written only through [`set_layer`](Mercury::set_layer) [..].
    #[resolve_into]
    layer: Layer,
}
```

The root's `#[bind(..)]` gains the hide, and `Default` gains the fields. Bind, after:

```rust
#[bind(
    Foregrounded => record_front_app,
    Quit => quit,
    OverlayTimeout => hide_overlay,
    AnyModifierKey => track_modifier,
    AnyNonModifierKey => maybe_pass_through,
)]
```

`Default`, after:

```rust
impl Default for Mercury {
    fn default() -> Self {
        Self {
            foreground: Foreground::default(),
            held: HeldModifiers::default(),
            overlay: None,
            next_overlay_id: OverlayId::default(),
            layer: Layer::Typing(TypingLayer::new()),
        }
    }
}
```

`with_layer` uses `..Self::default()`, so it needs no change.

### the root methods

New on `impl Mercury`, beside `handle` and `set_layer`:

```rust
/// Show the active layer's overlay and arm its hide timer. Reassigning the field drops any
/// previous guard, cancelling a still-pending timer, so a second `o` supersedes.
#[must_use = "the returned effects show the overlay and schedule its hide"]
pub fn show_overlay(&mut self) -> Vec<MercuryEffect> {
    let content = self.layer.get_overlay_content();
    let id = self.next_overlay_id;
    self.next_overlay_id = id.next();
    let (timer, effect) = timer_effect_and_guard(
        OVERLAY_DWELL,
        MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id }),
    );
    self.overlay = Some(ShownOverlay { id, timer });
    vec![MercuryEffect::ShowOverlay(content), MercuryEffect::Timer(effect)]
}

/// The overlay's hide timer fired: hide it only if the showing still up was armed with `id`. A
/// superseded showing's timer holds an older id, so the stale event is ignored.
#[must_use = "the returned effect hides the overlay"]
pub fn hide_overlay(&mut self, id: OverlayId) -> Vec<MercuryEffect> {
    if matches!(&self.overlay, Some(shown) if shown.id == id) {
        self.overlay = None;
        vec![MercuryEffect::HideOverlay]
    } else {
        Vec::new()
    }
}
```

`set_layer` hides the overlay on any transition. Before:

```rust
pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
    let into = into.into();
    let before_passthrough = self.layer.is_passthrough();
    let after_passthrough = into.is_passthrough();
    self.layer = into;
    match (before_passthrough, after_passthrough) {
        (true, false) => self.held.close(),
        (false, true) => self.held.open(),
        _ => Vec::new(),
    }
}
```

after:

```rust
pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
    let into = into.into();
    let before_passthrough = self.layer.is_passthrough();
    let after_passthrough = into.is_passthrough();
    self.layer = into;
    let mut effects = match (before_passthrough, after_passthrough) {
        (true, false) => self.held.close(),
        (false, true) => self.held.open(),
        _ => Vec::new(),
    };
    // Leaving a layer hides any overlay. Taking it drops the guard, cancelling the pending timer;
    // this is synchronous, so it needs no id check.
    if self.overlay.take().is_some() {
        effects.push(MercuryEffect::HideOverlay);
    }
    effects
}
```

`state/mod.rs` needs `OverlayId`, `OverlayTimeout`, and `OverlayTimeoutEvent` in its `use crate::{..}` list (the struct, the `#[bind]`, and `show_overlay` name them).

### the handlers

New `crates/mercury/src/handlers/overlay.rs`:

```rust
//! Showing and hiding the overlay: `o` in a non-typing layer shows it, its timeout hides it.

use bind::Node;
use laserbeam::Ascend;

use crate::state::{Mercury, MercuryPath};
use crate::{MercuryEffect, OverlayTimeoutEvent};

/// `o` in a non-typing layer: show the active layer's overlay and arm its hide timer.
///
/// Generic over the event and the path, so every non-typing layer binds it from its own node.
pub(crate) fn show_overlay<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().show_overlay()
}

/// The overlay's hide timer fired. Bound at the root, so it fires from whatever layer is active;
/// the id it carries is matched against the showing still up.
pub(crate) fn hide_overlay(
    ev: &OverlayTimeoutEvent,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.hide_overlay(ev.id)
}
```

`crates/mercury/src/handlers/mod.rs` gains `mod overlay;` and `pub(crate) use overlay::*;`, in the alphabetical slots.

### the bindings

Each non-typing layer binds `o`. `state/home.rs`, `nav.rs`, `resize.rs`, and `app.rs` each add one line to their `#[bind(..)]`. `home.rs`, after:

```rust
#[bind(
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyO.down() => show_overlay,
    Key::KeyQ.down() => quit,
)]
```

`nav.rs`, `resize.rs`, and `app.rs` add `Key::KeyO.down() => show_overlay,` to their own `#[bind(..)]` the same way. `typing.rs` does not: in typing, `o` falls through and is typed.

### the exports

`crates/mercury/src/lib.rs` adds `OverlayId`, `OverlayTimeout`, and `OverlayTimeoutEvent` to the `sources` re-export, and `OVERLAY_DWELL` to the `state` re-export.

### the effect-loop stub

`crates/mercury/src/main.rs`, `perform_effect`, add two arms so the match stays exhaustive. Change 3 replaces them:

```rust
MercuryEffect::ShowOverlay(text) => debug!(text, "show overlay (not yet drawn)"),
MercuryEffect::HideOverlay => debug!("hide overlay (not yet drawn)"),
```

### tests

`crates/mercury/tests/transitions.rs`. Add `OverlayId`, `OverlayTimeoutEvent`, and `OVERLAY_DWELL` to the `use mercury::{..}` list, and a helper beside `return_home_timer`:

```rust
// The effect `o` arms: the overlay's hide timer for showing `id`. Equality under `testing`
// compares the delay and fire event, so a rebuilt one matches what `show_overlay` produced.
fn overlay_hide_timer(id: OverlayId) -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(
        OVERLAY_DWELL,
        MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id }),
    );
    MercuryEffect::Timer(effect)
}
```

Cases:

```rust
#[test]
fn home_o_shows_the_overlay() {
    let mut m = home();
    assert_eq!(
        m.handle(&key(Key::KeyO)),
        Some(vec![
            MercuryEffect::ShowOverlay("n nav  r resize  t typing  i in-app  q quit"),
            overlay_hide_timer(OverlayId(0)),
        ])
    );
}

#[test]
fn nav_o_shows_the_nav_overlay() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyN));
    assert_eq!(
        m.handle(&key(Key::KeyO)),
        Some(vec![
            MercuryEffect::ShowOverlay("c chrome  f finder  g ghostty  z zed"),
            overlay_hide_timer(OverlayId(0)),
        ])
    );
}

#[test]
fn the_overlay_hides_after_the_dwell() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO)); // showing 0
    assert_eq!(
        m.handle(&MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id: OverlayId(0) })),
        Some(vec![MercuryEffect::HideOverlay])
    );
}

#[test]
fn a_stale_hide_timeout_does_not_hide_a_fresh_overlay() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO)); // showing 0
    let _ = m.handle(&key(Key::KeyO)); // reshow: showing 1, 0 superseded
    // Showing 0's timer fires late; it must not hide showing 1.
    assert_eq!(
        m.handle(&MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id: OverlayId(0) })),
        Some(vec![])
    );
    // Showing 1's timer does hide it.
    assert_eq!(
        m.handle(&MercuryEvent::OverlayTimeout(OverlayTimeoutEvent { id: OverlayId(1) })),
        Some(vec![MercuryEffect::HideOverlay])
    );
}

#[test]
fn changing_layers_hides_the_overlay() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO)); // overlay up
    // Entering nav cancels the hide timer and hides the window, on top of arming nav's own timer.
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        Some(vec![MercuryEffect::HideOverlay, return_home_timer()])
    );
}

#[test]
fn a_transition_with_no_overlay_does_not_hide() {
    let mut m = home();
    // No `o` first, so entering nav arms its timer and hides nothing.
    assert_eq!(m.handle(&key(Key::KeyN)), Some(vec![return_home_timer()]));
}

#[test]
fn o_in_typing_is_typed() {
    let mut m = Mercury::default(); // typing
    assert_eq!(m.handle(&key(Key::KeyO)), Some(passed(Key::KeyO)));
}
```

`resize.rs` and `app.rs` get the same `o`-shows-its-hint case as `nav`. The existing transition tests are untouched: a transition hides only when an overlay is up, which they never make.

At this point `o` shows and hides the overlay at the model level, the effects are asserted, and a run logs the show/hide. Nothing draws yet.

## change 3: the `freddie_overlay` panel and the effect loop

A borderless panel that floats above everything and shows centered text, driven by the two effects.

### the crate

New `crates/freddie_overlay/Cargo.toml`:

```toml
[package]
name = "freddie_overlay"
description = "A borderless macOS overlay panel for freddie."
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
dispatch2 = "0.2"
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = [
    "NSColor", "NSControl", "NSFont", "NSPanel", "NSResponder", "NSScreen",
    "NSText", "NSTextField", "NSView", "NSWindow",
] }
objc2-foundation = { version = "0.3", features = ["NSGeometry", "NSString"] }
tracing = "0.1"

# Not `workspace = true`: the workspace forbids `unsafe_code`, and `forbid` cannot be relaxed from
# inside the crate. Every AppKit call is unsafe and allowed at its site with a SAFETY comment.
[lints.rust]
unsafe_code = "deny"

[lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery = { level = "deny", priority = -1 }
cargo = { level = "deny", priority = -1 }
multiple_crate_versions = "allow"
cargo_common_metadata = "allow"
```

Add `"crates/freddie_overlay",` to the workspace `members` in the root `Cargo.toml`.

### the panel

New `crates/freddie_overlay/src/lib.rs`. `show` and `hide` are callable from any thread: they hop to the main thread by dispatching onto the main queue, which the `freddie_main_loop` run loop services. The panel is built lazily on the first `show` and reused, held in a main-thread-only `thread_local` (every dispatched block runs on main, so nothing else touches it).

```rust
//! A borderless overlay panel: centered text floating above everything, click-through.
//!
//! [`show`] and [`hide`] are callable from any thread; they marshal to the main thread, where
//! AppKit lives, by dispatching onto the main queue. It is serviced by the main run loop, so this
//! needs `freddie_main_loop` running and `NSApp` initialized, the same as the menu bar.
//!
//! macOS only.

use std::cell::RefCell;

use dispatch2::Queue;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreen, NSTextAlignment, NSTextField,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tracing::debug;

thread_local! {
    // The panel and its label, on the main thread. Only ever touched inside a dispatched block,
    // which always runs on main.
    static PANEL: RefCell<Option<(Retained<NSPanel>, Retained<NSTextField>)>> =
        const { RefCell::new(None) };
}

/// Show the overlay with `text`, from any thread.
pub fn show(text: &'static str) {
    Queue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("dispatched to the main queue");
        PANEL.with_borrow_mut(|slot| {
            let (panel, label) = slot.get_or_insert_with(|| build(mtm));
            // SAFETY: setting the label's text and repositioning the panel, on the main thread.
            unsafe {
                label.setStringValue(&NSString::from_str(text));
                center(panel, mtm);
                panel.orderFrontRegardless();
            }
        });
        debug!(text, "overlay shown");
    });
}

/// Hide the overlay, from any thread. A no-op if it is not up.
pub fn hide() {
    Queue::main().exec_async(|| {
        PANEL.with_borrow(|slot| {
            if let Some((panel, _)) = slot {
                // SAFETY: ordering the panel out, on the main thread.
                unsafe { panel.orderOut(None) };
            }
        });
        debug!("overlay hidden");
    });
}

/// Build the panel and its label. Borderless, non-activating, floating above menus, click-through,
/// on every space, with a large centered white label and no background.
fn build(mtm: MainThreadMarker) -> (Retained<NSPanel>, Retained<NSTextField>) {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(700.0, 120.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: the NSPanel designated initializer, on the main thread.
    let panel = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: standard panel configuration, on the main thread.
    unsafe {
        // Above normal windows and the menu bar. `NSScreenSaverWindowLevel` is 1000.
        panel.setLevel(1000);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
    }

    // SAFETY: a non-editable, non-bezeled label filling the content rect, on the main thread.
    let label = unsafe {
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setFrame(frame);
        label.setAlignment(NSTextAlignment::Center);
        label.setTextColor(Some(&NSColor::whiteColor()));
        label.setFont(Some(&NSFont::systemFontOfSize(48.0)));
        label.setDrawsBackground(false);
        label.setBezeled(false);
        label.setEditable(false);
        label.setSelectable(false);
        label
    };
    // SAFETY: installing the label as the panel's content view, on the main thread.
    unsafe { panel.setContentView(Some(&label)) };
    (panel, label)
}

/// Center the panel horizontally, a third of the way up the main screen's visible area.
///
/// # Safety
///
/// The caller must be on the main thread and hold `mtm`.
unsafe fn center(panel: &NSPanel, mtm: MainThreadMarker) {
    let Some(screen) = NSScreen::mainScreen(mtm) else {
        return;
    };
    // SAFETY: reading the screen's frame and moving the panel, on the main thread.
    unsafe {
        let vis = screen.visibleFrame();
        let size = panel.frame().size;
        let x = vis.origin.x + (vis.size.width - size.width) / 2.0;
        let y = vis.origin.y + vis.size.height / 3.0;
        panel.setFrameOrigin(NSPoint::new(x, y));
    }
}
```

### wiring it into mercury

`crates/mercury/Cargo.toml` adds `freddie_overlay = { path = "../freddie_overlay" }`.

`crates/mercury/src/main.rs`, `perform_effect`, replace the change-2 stubs:

```rust
MercuryEffect::ShowOverlay(text) => freddie_overlay::show(text),
MercuryEffect::HideOverlay => freddie_overlay::hide(),
```

The panel builds on the first `show` on the main thread and reuses thereafter; `hide` orders it out; a superseded `show` (a second `o`, or a new layer's hint) updates the same panel's text. Nothing else in `main.rs` changes: the overlay lives on the main thread, driven only by these two effects.

## open questions

- The hint strings are hand-written per layer. Deriving them from the layer's `#[bind(..)]` set would keep them in step with the bindings automatically; that needs a way to read a node's triggers, which `bind` does not expose yet.
- The panel styling (size, font, position, a background card) is a first pass, not tuned.
- Whether the overlay should also show for layers reached without a keypress (a foreground event retargeting the in-app layer, say), which today never flashes because only `o` shows it.
