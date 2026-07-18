# a self-clearing overlay, shown by effects

The overlay is an external AppKit window, not model state. The model cannot own an AppKit window by RAII across threads (the state tree lives on the worker thread, AppKit is main-thread-only), so the overlay is controlled entirely by effects: `ShowOverlay(text)` puts it up, `HideOverlay` takes it down. There is no overlay content or visibility in the tree.

`o` in any non-typing layer shows that layer's keymap hint and arms a hide timer. The overlay hides when you change layers, or after the dwell. One binding each side:

- `o => toggle_overlay` in every non-typing layer. `o` is a keystroke, so binding it at the root would fire in typing too and you could never type the letter, which is why it is per-layer. Typing binds nothing at all now, so `o` falls to the root and is passed through like every other key.
- the hide timer's own firing, bound once at the root. The timeout is a timer event, not a keystroke, and hiding is not opt-in, so a single root binding covers every layer.

The overlay's trace in the model is one field on the root, `overlay: Option<TimerGuard>`, the guard for the pending hide of whatever is up. It is a single thing shown from any layer, so its state is a root concern, not a per-layer one (unlike the return-home timer, which each layer owns independently). The three paths:

- `toggle_overlay` sets the hide timer and stores its guard, or takes the overlay down when one is already up: `o` is the key you press to ask what is bound, so it is the key you press when you are done reading.
- `hide_overlay` takes the field: if one was up, push `HideOverlay`.
- `set_layer` does the same, so leaving a layer takes the overlay down with it.

An overlay already up can go stale, and that is fine. Its content is read once, when `o` shows it, so an in-app overlay keeps the keymap of whatever app was frontmost then; `record_front_app` changes the front app without a transition, and nothing hides or redraws it. The dwell takes it down, or the next `o` shows the current one. It is a hint you asked for, not a live view.

A superseded showing can still fire, and nothing here has to care. The binding names the guard the root holds, so a firing from a showing that was replaced matches no binding at all and the handler never runs. That is `refactors/past/timer-ids.md`, which landed: every timer shares one `TimerFired` event, and which timer fired is what the binding matches on.

## change 1: the overlay effects, content, and bindings

`o` shows the active layer's hint and sets the hide timer; its firing and every layer change hide it. Rendering is a `debug!` line here; change 2 makes it draw.

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

### no timeout trigger

The overlay needs neither an event nor a trigger of its own. Every timer fires `freddie::TimerFired`, and what tells one from another is which guard the firing came from, so the root binds the guard it is holding and nothing else is added to `sources.rs`, `MercuryTrigger`, or `MercuryEvent`.

### the layer's content

`crates/mercury/src/state/mod.rs`. Add the dwell next to the return-home timeout:

```rust
/// How long the overlay stays up before its hide timer fires.
pub const OVERLAY_DWELL: Duration = Duration::from_secs(10);
```

Each layer's keymap is a file, not a string literal. A table written with `concat!` and `\n` escapes cannot be read as the table it is, and the whole point is that the columns line up; in a file, what you see is what the overlay draws.

They live in one folder beside the layers, `crates/mercury/src/state/overlays/`, one file per keymap:

```
crates/mercury/src/state/
  overlays/
    home.txt
    nav.txt
    resize.txt
    typing.txt
    chrome.txt
    ghostty.txt
    inapp.txt
  home.rs
  nav.rs
  ..
```

`inapp.txt` is for an app with no bindings of its own. `nav.txt`:

```
  NAV
  ────────────────────
  c    chrome
  f    finder
  g    ghostty
  z    zed
  esc  home
```

Each module includes its own at compile time, so a missing file is a build error rather than a blank overlay, and nothing reads the disk at runtime. `nav.rs`, beside its `#[bind(..)]`:

```rust
/// The keymap the overlay shows for this layer. Beside the bindings it describes, so the two are
/// changed together or the drift is obvious.
pub(crate) const OVERLAY: &str = include_str!("overlays/nav.txt");
```

`home.rs`, `resize.rs`, and `typing.rs` do the same. `app.rs` carries one per app, since the in-app layer's bindings are the front app's:

```rust
pub(crate) const CHROME_OVERLAY: &str = include_str!("overlays/chrome.txt");
pub(crate) const GHOSTTY_OVERLAY: &str = include_str!("overlays/ghostty.txt");
/// For an app with no bindings of its own: the in-app layer's own keys and nothing more.
pub(crate) const INAPP_OVERLAY: &str = include_str!("overlays/inapp.txt");
```

`impl Layer`, beside `is_passthrough`, names them:

```rust
/// The keymap the overlay shows for this layer, read when `o` shows it.
///
/// `app` is the confirmed front app, which only the in-app layer reads: its bindings are the
/// app's, so `i` in Ghostty and `i` in Chrome are different keymaps and showing one for the other
/// would be worse than showing nothing. Typing never binds `o`, so its arm is unreachable.
#[must_use]
pub const fn overlay_content(&self, app: App) -> &'static str {
    match self {
        Self::Home(_) => home::OVERLAY,
        Self::Nav(_) => nav::OVERLAY,
        Self::Resize(_) => resize::OVERLAY,
        Self::InApp(_) => app::overlay_for(app),
        Self::Typing(_) => typing::OVERLAY,
    }
}
```

and `app.rs` maps the app to its own:

```rust
/// The keymap for the in-app layer while `app` is frontmost.
#[must_use]
pub(crate) const fn overlay_for(app: App) -> &'static str {
    match app {
        App::Chrome => CHROME_OVERLAY,
        App::Ghostty => GHOSTTY_OVERLAY,
        App::Zed | App::Other => INAPP_OVERLAY,
    }
}
```

A file ends with a newline, which would draw as a blank last row. The panel trims it when it sets the label (change 2), rather than the model trimming: `str::trim_end` is not `const`, and this is a presentation concern anyway.

### the root state

The showing up, if any, is one field. Add the struct near `Mercury`:

```rust
The field is the guard itself: `Some` means an overlay is up, and dropping it cancels that
showing's pending hide.
```

The `Mercury` struct, before:

```rust
pub struct Mercury {
    /// The frontmost app and whether a nav is in flight. See [`Foreground`].
    pub foreground: Foreground,
    /// The state the passthrough (typing) behavior needs. See [`TypingState`].
    pub typing_state: TypingState,
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
    /// The state the passthrough (typing) behavior needs. See [`TypingState`].
    pub typing_state: TypingState,
    /// The overlay currently up, if any: the guard for its pending hide. The overlay is an
    /// external window driven by effects, so this is its only trace in the model, held at the root
    /// because there is one overlay across all layers. The root's binding names it, so a firing
    /// from a showing that was replaced matches nothing.
    overlay: Option<TimerGuard>,
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
    JkTimeout => jk_timeout,
    // Only the showing that is up: a dwell from one already replaced matches nothing.
    |mercury_path| mercury_path.overlay.as_ref().map(TimerGuard::trigger) => hide_overlay,
    AnyKey => maybe_pass_through,
)]
```

`Default`, after:

```rust
impl Default for Mercury {
    fn default() -> Self {
        Self {
            foreground: Foreground::default(),
            typing_state: TypingState::default(),
            overlay: None,
            layer: Layer::Typing(TypingLayer::new()),
        }
    }
}
```

`with_layer` uses `..Self::default()`, so it needs no change.

### the root methods

New on `impl Mercury`, beside `handle` and `set_layer`:

```rust
/// Show the active layer's keymap, or take it down if it is already up.
///
/// One of the two writers of `overlay`, which is private for the reason `layer` is: the effects a
/// change implies come back from the method that made it, and `#[must_use]` is what stops them
/// being dropped. `refactors/pending/drop-emits-effects.md` is the general form; nothing here
/// waits on it.
#[must_use = "the returned effects put the overlay up or take it down"]
pub fn toggle_overlay(&mut self) -> Vec<MercuryEffect> {
    if self.overlay.is_some() {
        return self.hide_overlay();
    }
    let content = self.layer.overlay_content(self.foreground.app());
    let (guard, effect) =
        timer_effect_and_guard(OVERLAY_DWELL, |id| MercuryEvent::Timer(TimerFired(id)));
    self.overlay = Some(guard);
    vec![MercuryEffect::ShowOverlay(content), MercuryEffect::Timer(effect)]
}

/// Hide the overlay if one is up. Both the dwell firing and a layer change come through here, and
/// taking the field drops the guard, cancelling a hide that has not fired yet.
#[must_use = "the returned effect hides the overlay"]
pub fn hide_overlay(&mut self) -> Vec<MercuryEffect> {
    if self.overlay.take().is_some() {
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
    self.typing_state.jk = KeySequence::new(JK, Some(JK_TIMEOUT));
    let mut effects = match (before_passthrough, after_passthrough) {
        (true, false) => self.typing_state.held.close(),
        (false, true) => self.typing_state.held.open(),
        _ => Vec::new(),
    };
    effects.push(MercuryEffect::ShowLayer(self.layer.name()));
    effects
}
```

after:

```rust
pub fn set_layer(&mut self, into: impl Into<Layer>) -> Vec<MercuryEffect> {
    let into = into.into();
    let before_passthrough = self.layer.is_passthrough();
    let after_passthrough = into.is_passthrough();
    self.layer = into;
    self.typing_state.jk = KeySequence::new(JK, Some(JK_TIMEOUT));
    let mut effects = match (before_passthrough, after_passthrough) {
        (true, false) => self.typing_state.held.close(),
        (false, true) => self.typing_state.held.open(),
        _ => Vec::new(),
    };
    // Leaving a layer takes the overlay with it, cancelling its pending hide.
    effects.append(&mut self.hide_overlay());
    effects.push(MercuryEffect::ShowLayer(self.layer.name()));
    effects
}
```

`state/mod.rs` needs `TimerFired` and `TimerGuard` from `freddie`; it adds nothing to its `use crate::{..}`.

### the handlers

New `crates/mercury/src/handlers/overlay.rs`:

```rust
//! Showing and hiding the overlay: `o` in a non-typing layer shows it, its timeout hides it.

use bind::Node;
use laserbeam::Ascend;

use freddie::TimerFired;

use crate::state::{Mercury, MercuryPath};
use crate::MercuryEffect;

/// `o` in a non-typing layer: show the active layer's overlay and arm its hide timer.
///
/// Generic over the event and the path, so every non-typing layer binds it from its own node.
pub(crate) fn toggle_overlay<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    node.parent.ascend().toggle_overlay()
}

/// The overlay's hide timer fired. Bound at the root, so it fires from whatever layer is active,
/// and only for the showing still up: the binding matches the guard the root holds.
pub(crate) fn hide_overlay(
    _ev: &TimerFired,
    node: Node<&mut Mercury, ()>,
) -> Vec<MercuryEffect> {
    node.parent.hide_overlay()
}
```

`crates/mercury/src/handlers/mod.rs` gains `mod overlay;` and `pub(crate) use overlay::*;`, in the alphabetical slots.

### the bindings

Each non-typing layer binds `o`. `state/home.rs`, `nav.rs`, `resize.rs`, and `app.rs` each add one line to their `#[bind(..)]`. `home.rs`, after:

```rust
#[bind(
    Key::Escape.down() => to_home,
    Key::KeyN.down() => to_nav,
    Key::KeyR.down() => to_resize,
    Key::KeyT.down() => to_typing,
    Key::KeyI.down() => to_inapp,
    Key::KeyO.down() => toggle_overlay,
    Key::KeyQ.down() => quit,
)]
```

`nav.rs`, `resize.rs`, and `app.rs` add `Key::KeyO.down() => toggle_overlay,` to their own `#[bind(..)]` the same way. `typing.rs` binds nothing and gains nothing: in typing, `o` falls to the root and is typed.

### the exports

`crates/mercury/src/lib.rs` adds `OVERLAY_DWELL` to the `state` re-export, and nothing to the `sources` one.

### the effect-loop stub

`crates/mercury/src/main.rs`, `perform_effect`, add two arms so the match stays exhaustive. Change 3 replaces them:

```rust
MercuryEffect::ShowOverlay(text) => debug!(text, "show overlay (not yet drawn)"),
MercuryEffect::HideOverlay => debug!("hide overlay (not yet drawn)"),
```

### tests

`crates/mercury/tests/transitions.rs`. Add `OVERLAY_DWELL` to the `use mercury::{..}` list, and a helper beside `return_home_timer`, built with the `fired` and `timer_id` helpers that are already there:

```rust
// The effect `o` produces: the overlay's hide timer. Equality under `testing` compares the delay
// and the fire event, and a firing compares equal whatever its id, so a rebuilt one matches.
fn overlay_hide_timer() -> MercuryEffect {
    let (_guard, effect) = freddie::timer_effect_and_guard(OVERLAY_DWELL, fired);
    MercuryEffect::Timer(effect)
}
```

Cases. The content is asserted by its first line rather than in full, so re-wording a keymap does
not rewrite the test table:

```rust
#[test]
fn o_shows_the_layers_overlay() {
    for (enter, heading) in [
        (None, "  HOME"),
        (Some(Key::KeyN), "  NAV"),
        (Some(Key::KeyR), "  RESIZE"),
    ] {
        let mut m = home();
        if let Some(k) = enter {
            let _ = m.handle(&key(k));
        }
        let effects = m.handle(&key(Key::KeyO)).expect("o is bound");
        let [MercuryEffect::ShowOverlay(text), timer] = effects.as_slice() else {
            panic!("o shows the overlay and arms its hide: {effects:?}");
        };
        assert!(text.starts_with(heading), "{heading}: {text}");
        assert_eq!(*timer, overlay_hide_timer());
    }
}

#[test]
fn the_in_app_overlay_is_the_front_apps_keymap() {
    // The in-app layer's bindings are the app's, so its overlay has to be too.
    for (app, heading) in [
        (App::Chrome, "  CHROME"),
        (App::Ghostty, "  GHOSTTY"),
        (App::Zed, "  IN-APP"),
    ] {
        let mut m = home();
        let _ = m.handle(&foreground(app));
        let _ = m.handle(&key(Key::KeyI));
        let effects = m.handle(&key(Key::KeyO)).expect("o is bound");
        let [MercuryEffect::ShowOverlay(text), _] = effects.as_slice() else {
            panic!("o shows the overlay: {effects:?}");
        };
        assert!(text.starts_with(heading), "{app:?}: {text}");
    }
}

#[test]
fn the_overlay_hides_after_the_dwell() {
    let mut m = home();
    let shown = m.handle(&key(Key::KeyO)).expect("o is bound");
    assert_eq!(
        m.handle(&fired(timer_id(&shown))),
        Some(vec![MercuryEffect::HideOverlay])
    );
    // And again matches nothing: the field was taken, so no binding names that guard.
    assert_eq!(m.handle(&fired(timer_id(&shown))), None);
}

#[test]
fn a_dwell_from_a_superseded_showing_matches_nothing() {
    // Two `o`s in a row: the first showing's dwell arrives after the second replaced it, and must
    // not take the live overlay down.
    let mut m = home();
    let first = timer_id(&m.handle(&key(Key::KeyO)).expect("o is bound"));
    let second = timer_id(&m.handle(&key(Key::KeyO)).expect("o is bound"));
    assert_ne!(first, second, "each showing sets its own dwell");

    assert_eq!(m.handle(&fired(first)), None);
    assert_eq!(
        m.handle(&fired(second)),
        Some(vec![MercuryEffect::HideOverlay])
    );
}

#[test]
fn changing_layers_hides_the_overlay() {
    let mut m = home();
    let _ = m.handle(&key(Key::KeyO)); // overlay up
    // Entering nav hides it, on top of arming nav's own timer and naming the layer.
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        Some(vec![
            MercuryEffect::HideOverlay,
            shows("Nav"),
            return_home_timer(),
        ])
    );
}

#[test]
fn a_transition_with_no_overlay_does_not_hide() {
    let mut m = home();
    // No `o` first, so entering nav arms its timer and hides nothing.
    assert_eq!(
        m.handle(&key(Key::KeyN)),
        Some(vec![shows("Nav"), return_home_timer()])
    );
}

#[test]
fn o_in_typing_is_typed() {
    let mut m = Mercury::default(); // typing
    assert_eq!(m.handle(&key(Key::KeyO)), Some(passed(Key::KeyO)));
}
```

The existing transition tests are untouched: a transition hides only when an overlay is up, which they never make.

Where `HideOverlay` lands relative to `ShowLayer` is what `set_layer` does: it appends the hide before pushing the name, so a transition out of a layer with an overlay up reads `[HideOverlay, ShowLayer(..), ..]`.

At this point `o` shows and hides the overlay at the model level, the effects are asserted, and a run logs the show/hide. Nothing draws yet.

## change 2: the `freddie_overlay` panel and the effect loop

A borderless panel that floats above everything and shows the keymap, driven by the two effects.

It is modelled on voice-mode's hammerspoon overlay (`~/code/voicemode/hammerspoon/ui.lua`, `showLayerOverlay`), which is the shape to match: a dark rounded panel against the right edge, vertically centered, its text monospaced and left-aligned, sized to its content rather than fixed.

Monospace is not decoration. The content is a fixed-width table written with spaces (change 2), so a proportional font would break every column in it.

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
dispatch2 = "0.3"
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = [
    "NSColor", "NSControl", "NSFont", "NSPanel", "NSResponder", "NSScreen",
    "NSText", "NSTextField", "NSView", "NSWindow",
] }
objc2-foundation = { version = "0.3", features = ["NSGeometry", "NSString"] }
# The rounded dark background is a layer-backed view's `CALayer`.
objc2-quartz-core = { version = "0.3", features = ["CALayer"] }
objc2-core-foundation = { version = "0.3", features = ["CGColor"] }
tracing = "0.1"

# Not `workspace = true` only for the clippy relaxations below; the crate needs no `unsafe` at
# all, because objc2 0.6 exposes every call it makes as safe.
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

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CGColor;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tracing::debug;

thread_local! {
    // The panel and its label, on the main thread. Only ever touched inside a dispatched block,
    // which always runs on main.
    static PANEL: RefCell<Option<(Retained<NSPanel>, Retained<NSTextField>)>> =
        const { RefCell::new(None) };
}

/// Show the overlay with `text`, from any thread.
///
/// The panel is sized to the text, so a keymap with more rows makes a taller panel rather than a
/// clipped one.
pub fn show(text: &'static str) {
    DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("dispatched to the main queue");
        PANEL.with_borrow_mut(|slot| {
            let (panel, label) = slot.get_or_insert_with(|| build(mtm));
            // SAFETY: setting the label's text, resizing to fit it, and placing the panel, on the
            // main thread.
            unsafe {
                // Trimmed: each keymap is a file, and a file ends with a newline, which would
                // draw as a blank last row.
                label.setStringValue(&NSString::from_str(text.trim_end()));
                label.sizeToFit();
                resize_to_label(panel, label);
                place(panel, mtm);
                panel.orderFrontRegardless();
            }
        });
        debug!(text, "overlay shown");
    });
}

/// Hide the overlay, from any thread. A no-op if it is not up.
pub fn hide() {
    DispatchQueue::main().exec_async(|| {
        PANEL.with_borrow(|slot| {
            if let Some((panel, _)) = slot {
                // SAFETY: ordering the panel out, on the main thread.
                unsafe { panel.orderOut(None) };
            }
        });
        debug!("overlay hidden");
    });
}

/// Build the panel, its rounded dark background, and its label. Borderless, non-activating,
/// floating above menus, click-through, on every space.
fn build(mtm: MainThreadMarker) -> (Retained<NSPanel>, Retained<NSTextField>) {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
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
        // Above normal windows and the menu bar. `NSScreenSaverWindowLevel` is 1000, and
        // `NSWindowLevel` is an `NSInteger`, so the literal is the level.
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

    // SAFETY: a layer-backed container drawing the rounded dark background, on the main thread.
    let container = unsafe {
        let view = NSView::initWithFrame(mtm.alloc(), frame);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&CGColor::new_generic_gray(0.0, 0.85)));
            layer.setCornerRadius(10.0);
        }
        view
    };

    // SAFETY: a non-editable, non-bezeled, multi-line monospaced label, on the main thread.
    let label = unsafe {
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setAlignment(NSTextAlignment::Left);
        label.setTextColor(Some(&NSColor::whiteColor()));
        // Monospaced, because the content is a table laid out with spaces.
        label.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
            FONT_SIZE, 0.0,
        )));
        label.setUsesSingleLineMode(false);
        label.setMaximumNumberOfLines(0);
        label.setDrawsBackground(false);
        label.setBezeled(false);
        label.setEditable(false);
        label.setSelectable(false);
        label
    };
    // SAFETY: installing the label in the container and the container in the panel, on main.
    unsafe {
        container.addSubview(&label);
        panel.setContentView(Some(&container));
    }
    (panel, label)
}

/// Grow the panel to the label's fitted size plus the padding, and inset the label inside it.
///
/// # Safety
///
/// The caller must be on the main thread.
unsafe fn resize_to_label(panel: &NSPanel, label: &NSTextField) {
    // SAFETY: reading the fitted label and resizing the panel and its views, on the main thread.
    unsafe {
        let text = label.frame().size;
        let size = NSSize::new(text.width + PADDING * 2.0, text.height + PADDING * 2.0);
        panel.setContentSize(size);
        if let Some(container) = panel.contentView() {
            container.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        }
        label.setFrameOrigin(NSPoint::new(PADDING, PADDING));
    }
}

/// Put the panel against the right edge of the main screen, vertically centered.
///
/// # Safety
///
/// The caller must be on the main thread and hold `mtm`.
unsafe fn place(panel: &NSPanel, mtm: MainThreadMarker) {
    let Some(screen) = NSScreen::mainScreen(mtm) else {
        return;
    };
    // SAFETY: reading the screen's visible frame and moving the panel, on the main thread.
    unsafe {
        let vis = screen.visibleFrame();
        let size = panel.frame().size;
        let x = vis.origin.x + vis.size.width - size.width - MARGIN;
        let y = vis.origin.y + (vis.size.height - size.height) / 2.0;
        panel.setFrameOrigin(NSPoint::new(x, y));
    }
}
```

The three numbers it reads, at the top of the file:

```rust
/// The monospaced type size the keymap is drawn at.
const FONT_SIZE: f64 = 15.0;
/// Space between the text and the panel's edge.
const PADDING: f64 = 20.0;
/// Space between the panel and the screen's right edge.
const MARGIN: f64 = 20.0;
```

### wiring it into mercury

`crates/mercury/Cargo.toml` adds `freddie_overlay = { path = "../freddie_overlay" }`.

`crates/mercury/src/main.rs`, `perform_effect`, replace the change-1 stubs:

```rust
MercuryEffect::ShowOverlay(text) => freddie_overlay::show(text),
MercuryEffect::HideOverlay => freddie_overlay::hide(),
```

The panel builds on the first `show` on the main thread and reuses thereafter; `hide` orders it out; a superseded `show` (a second `o`, or a new layer's hint) updates the same panel's text. Nothing else in `main.rs` changes: the overlay lives on the main thread, driven only by these two effects.

## open questions

- The panel styling (size, font, position, a background card) is a first pass, not tuned.

