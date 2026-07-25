//! A borderless overlay panel: a translucent dark card of monospaced text, floating above
//! everything.
//!
//! It cannot be interacted with. The mouse passes through it (`setIgnoresMouseEvents`), it never
//! takes focus (`NonactivatingPanel`), and it stays put when the app it covers is deactivated, so
//! it reads as part of the screen rather than as a window.
//!
//! [`overlay`] builds one on the main thread and returns the [`Overlay`] that owns the panel
//! beside the first [`OverlaySink`]. Dropping the overlay closes the panel. The sink is `Send`
//! and `Clone`: [`OverlaySink::show`] and [`OverlaySink::hide`] are callable from any thread and
//! send over a [`freddie_main_loop::WakingSender`], which wakes the main run loop so
//! [`Overlay::pump`] (called from `on_wake`) applies the change. Needs `freddie_main_loop`
//! running and `NSApp` initialized, the same as the menu bar.
//!
//! More than one overlay is fine: each handle drives its own panel, and dropping one leaves the
//! others alone.
//!
//! macOS only.

use std::sync::mpsc::Receiver;

use freddie_main_loop::{MainWaker, WakingSender};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_graphics::CGColor;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tracing::debug;

/// The monospaced type size the keymap is drawn at. Big enough to read across the room, since it
/// is a glance, not a document.
const FONT_SIZE: f64 = 36.0;
/// Space between the text and the panel's edge.
const PADDING: f64 = 32.0;
/// Space between the panel and the screen's right edge.
const MARGIN: f64 = 20.0;
/// How opaque the card behind the text is. It is a hint you read past, not a window: low enough to
/// see what is underneath, high enough for white monospaced text to stay legible over anything.
const BACKGROUND_ALPHA: f64 = 0.7;

/// One overlay's panel and the label it draws.
struct Panel {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
}

/// What a sink asks its overlay to do. Sent over the channel, drained on the main thread.
enum OverlayMsg {
    /// Show with this text, sizing the panel to it.
    Show(String),
    /// Take the panel off the screen; the panel stays built.
    Hide,
}

/// The overlay's lifetime. Holding it keeps the panel built; dropping it closes the panel.
///
/// `!Send`, because the panel is: `Retained<NSPanel>` stays on the thread that built it. It stays
/// where [`overlay`] built it, like `freddie_menu_bar`'s `MenuBar`.
///
/// It does not show anything. The [`OverlaySink`] returned beside it is what a worker uses.
pub struct Overlay {
    /// The panel this overlay owns. `Retained<NSPanel>` is not `Send`, which keeps `Overlay` on
    /// the thread that built it without a `PhantomData`.
    panel: Panel,
    /// Drained by [`Overlay::pump`] on the main thread when the loop wakes. The overlay holds only
    /// this end of the channel; the sinks hold the senders.
    message_receiver: Receiver<OverlayMsg>,
}

/// The handle showing and hiding go through. `Send` and `Clone`, so any thread can hold one; the
/// panel it drives is on the main thread, reached by sending rather than by touching it.
///
/// Safe to keep past its [`Overlay`]: once the overlay is dropped the receiver is gone, and a send
/// is a harmless error, which is what hiding an already-gone overlay would have been.
#[derive(Clone)]
pub struct OverlaySink {
    message_sender: WakingSender<OverlayMsg>,
}

impl std::fmt::Debug for OverlaySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlaySink").finish_non_exhaustive()
    }
}

/// Build an overlay panel, hidden, and return the handle that owns it beside the first sink.
///
/// Eagerly, not on first show: the panel is present for the whole life of the [`Overlay`], so
/// showing never has to build one and a keystroke puts an existing panel on screen.
///
/// # Panics
///
/// If called off the main thread, where `NSPanel` cannot be built.
#[must_use]
pub fn overlay(waker: &MainWaker) -> (Overlay, OverlaySink) {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let (message_sender, message_receiver) = waker.channel();
    debug!("overlay built");
    (
        Overlay {
            panel: build(mtm),
            message_receiver,
        },
        OverlaySink { message_sender },
    )
}

impl Overlay {
    /// Apply every queued show/hide to the panel. Call on the main thread, from `on_wake`.
    ///
    /// # Panics
    ///
    /// If called off the main thread, where the panel cannot be touched.
    pub fn pump(&self) {
        let mtm = MainThreadMarker::new().expect("Overlay::pump must run on the main thread");
        let Panel { panel, label } = &self.panel;
        for msg in self.message_receiver.try_iter() {
            match msg {
                OverlayMsg::Show(text) => {
                    // Trimmed: each keymap is a file, and a file ends with a newline, which would
                    // draw as a blank last row.
                    label.setStringValue(&NSString::from_str(text.trim_end()));
                    label.sizeToFit();
                    resize_to_label(panel, label);
                    place(panel, mtm);
                    panel.orderFrontRegardless();
                    debug!(text, "overlay shown");
                }
                OverlayMsg::Hide => {
                    panel.orderOut(None);
                    debug!("overlay hidden");
                }
            }
        }
    }
}

impl Drop for Overlay {
    /// Gives the panel back.
    ///
    /// Dropping the `Retained` alone would not: `AppKit`'s window list holds its own reference to a
    /// window, so the panel would stay alive, and stay on screen, with nothing on this side able to
    /// reach it. `close` takes it off the screen and off that list, so no `orderOut` is needed
    /// first, and `build` cleared `releasedWhenClosed` so the release is ours to perform.
    fn drop(&mut self) {
        self.panel.panel.close();
        debug!("overlay closed");
    }
}

impl OverlaySink {
    /// Show the overlay with `text`, from any thread. The send wakes the main loop, so `pump` runs
    /// and the panel updates at once.
    ///
    /// The panel is sized to the text, so a keymap with more rows makes a taller panel rather than
    /// a clipped one.
    pub fn show(&self, text: String) {
        let _ = self.message_sender.send(OverlayMsg::Show(text));
    }

    /// Hide the overlay, from any thread. A no-op if it is not up.
    ///
    /// The panel stays built, because it will be shown again: the next show puts an existing panel
    /// on screen rather than constructing one.
    pub fn hide(&self) {
        let _ = self.message_sender.send(OverlayMsg::Hide);
    }
}

/// Build the panel, its rounded dark background, and its label. Borderless, non-activating,
/// floating above menus, click-through, on every space.
fn build(mtm: MainThreadMarker) -> Panel {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: the NSPanel designated initializer, on the main thread.
    let panel = {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: standard panel configuration, on the main thread.
    {
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
    let container = {
        let view = NSView::initWithFrame(mtm.alloc(), frame);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&CGColor::new_generic_gray(0.0, BACKGROUND_ALPHA)));
            layer.setCornerRadius(10.0);
        }
        view
    };

    // SAFETY: a non-editable, non-bezeled, multi-line monospaced label, on the main thread.
    let label = {
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
    {
        container.addSubview(&label);
        panel.setContentView(Some(&container));
    }
    // SAFETY: setting the panel's own release policy, on the main thread, before anything else
    // holds it.
    #[expect(unsafe_code)]
    unsafe {
        // Ours to release, not AppKit's. `NSWindow` defaults to releasing itself when closed,
        // which would have `Overlay::drop`'s `close` release a panel the `Retained` still holds.
        panel.setReleasedWhenClosed(false);
    }
    Panel { panel, label }
}

/// Grow the panel to the label's fitted size plus the padding, and inset the label inside it.
fn resize_to_label(panel: &NSPanel, label: &NSTextField) {
    // SAFETY: reading the fitted label and resizing the panel and its views, on the main thread.
    {
        let text = label.frame().size;
        let size = NSSize::new(
            PADDING.mul_add(2.0, text.width),
            PADDING.mul_add(2.0, text.height),
        );
        panel.setContentSize(size);
        if let Some(container) = panel.contentView() {
            container.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        }
        label.setFrameOrigin(NSPoint::new(PADDING, PADDING));
    }
}

/// Put the panel against the right edge of the main screen, vertically centered.
fn place(panel: &NSPanel, mtm: MainThreadMarker) {
    let Some(screen) = NSScreen::mainScreen(mtm) else {
        return;
    };
    // SAFETY: reading the screen's visible frame and moving the panel, on the main thread.
    {
        let vis = screen.visibleFrame();
        let size = panel.frame().size;
        let x = vis.origin.x + vis.size.width - size.width - MARGIN;
        let y = vis.origin.y + (vis.size.height - size.height) / 2.0;
        panel.setFrameOrigin(NSPoint::new(x, y));
    }
}
