//! A borderless overlay panel: a translucent dark card of monospaced text, floating above
//! everything.
//!
//! It cannot be interacted with. The mouse passes through it (`setIgnoresMouseEvents`), it never
//! takes focus (`NonactivatingPanel`), and it stays put when the app it covers is deactivated, so
//! it reads as part of the screen rather than as a window.
//!
//! [`overlay`] builds one on the main thread and returns the [`Overlay`] that owns it. Dropping
//! that closes the panel and gives it back. [`Overlay::sink`] hands out an [`OverlaySink`], which
//! is `Send`: [`OverlaySink::show`] and [`OverlaySink::hide`] are callable from any thread and
//! marshal to the main thread, where `AppKit` lives, by dispatching onto the main queue. It is
//! serviced by the main run loop, so this needs `freddie_main_loop` running and `NSApp`
//! initialized, the same as the menu bar.
//!
//! More than one overlay is fine: each handle drives its own panel, and dropping one leaves the
//! others alone.
//!
//! macOS only.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;

use dispatch2::DispatchQueue;
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

thread_local! {
    /// Every overlay built on this thread, by id.
    ///
    /// Private, and only an [`Overlay`] or [`OverlaySink`] can reach an entry: a block dispatched
    /// to the main queue has to be `'static` and `Send`, so it cannot carry an `NSPanel` and looks
    /// one up here instead. An id is in the table between [`overlay`] building it and the handle
    /// dropping.
    ///
    /// A table and not a single slot: nothing about a panel makes it the only one.
    static PANELS: RefCell<HashMap<OverlayId, Panel>> = RefCell::new(HashMap::new());

    /// Mints the next [`OverlayId`]. A plain `Cell` because overlays are only ever built on this
    /// thread.
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

/// One overlay's entry in `PANELS`. Ids are never reused within a run, so a sink outliving its
/// overlay cannot end up pointed at a later one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct OverlayId(u64);

/// One overlay's panel and the label it draws.
struct Panel {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
}

/// The overlay's lifetime. Holding it keeps the panel built; dropping it closes the panel.
///
/// `!Send`, because `Drop` reaches `PANELS`, and a `thread_local` reached from another thread is a
/// different table: a handle dropped off main would find no entry and leave the real panel on
/// screen. It stays where [`overlay`] built it, like `freddie_menu_bar`'s `MenuBar`.
///
/// It does not show anything. [`sink`](Overlay::sink) is what a worker uses.
pub struct Overlay {
    id: OverlayId,
    _main_thread_only: PhantomData<*const ()>,
}

/// The handle showing and hiding go through. Cheap to copy and `Send`, because it carries nothing:
/// [`show`](Self::show) and [`hide`](Self::hide) dispatch to the main queue and find the panel
/// there.
///
/// Safe to keep past its [`Overlay`]. The dispatched block finds no entry for its id and does
/// nothing, which is what hiding an already-hidden overlay would have done.
#[derive(Clone, Copy, Debug)]
pub struct OverlaySink {
    id: OverlayId,
}

/// Build an overlay panel, hidden, and return the handle that owns it.
///
/// Eagerly, not on first show: it is what keeps the entry present for the whole life of the
/// [`Overlay`], so showing never has to build one and a keystroke puts an existing panel on screen.
///
/// # Panics
///
/// If called off the main thread, where `NSPanel` cannot be built.
#[must_use]
pub fn overlay() -> Overlay {
    let mtm = MainThreadMarker::new().expect("overlay must be built on the main thread");
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        OverlayId(id)
    });
    PANELS.with_borrow_mut(|panels| panels.insert(id, build(mtm)));
    debug!(?id, "overlay built");
    Overlay {
        id,
        _main_thread_only: PhantomData,
    }
}

impl Overlay {
    /// A handle to show and hide through. Cheap to copy, `Send`, and safe to keep past the overlay
    /// itself.
    #[must_use]
    pub const fn sink(&self) -> OverlaySink {
        OverlaySink { id: self.id }
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
        PANELS.with_borrow_mut(|panels| {
            if let Some(panel) = panels.remove(&self.id) {
                panel.panel.close();
            }
        });
        debug!(id = ?self.id, "overlay closed");
    }
}

impl OverlaySink {
    /// Show the overlay with `text`, from any thread.
    ///
    /// The panel is sized to the text, so a keymap with more rows makes a taller panel rather than
    /// a clipped one.
    ///
    /// # Panics
    ///
    /// If the dispatched block somehow runs off the main queue, which would mean libdispatch
    /// handed the main queue's work to another thread.
    pub fn show(&self, text: String) {
        let id = self.id;
        DispatchQueue::main().exec_async(move || {
            let mtm = MainThreadMarker::new().expect("dispatched to the main queue");
            // Shared, not mutable: every setter here takes `&self`, and the table itself is only
            // written by `overlay` and `Overlay::drop`.
            PANELS.with_borrow(|panels| {
                let Some(Panel { panel, label }) = panels.get(&id) else {
                    return;
                };
                // Trimmed: each keymap is a file, and a file ends with a newline, which would
                // draw as a blank last row.
                label.setStringValue(&NSString::from_str(text.trim_end()));
                label.sizeToFit();
                resize_to_label(panel, label);
                place(panel, mtm);
                panel.orderFrontRegardless();
            });
            debug!(text, "overlay shown");
        });
    }

    /// Hide the overlay, from any thread. A no-op if it is not up.
    ///
    /// The panel stays built, because it will be shown again: the next show puts an existing panel
    /// on screen rather than constructing one.
    pub fn hide(&self) {
        let id = self.id;
        DispatchQueue::main().exec_async(move || {
            PANELS.with_borrow(|panels| {
                if let Some(panel) = panels.get(&id) {
                    panel.panel.orderOut(None);
                }
            });
            debug!("overlay hidden");
        });
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
