//! A borderless overlay panel: a translucent dark card of monospaced text, floating above
//! everything.
//!
//! It cannot be interacted with. The mouse passes through it (`setIgnoresMouseEvents`), it never
//! takes focus (`NonactivatingPanel`), and it stays put when the app it covers is deactivated, so
//! it reads as part of the screen rather than as a window.
//!
//! [`show`] and [`hide`] are callable from any thread; they marshal to the main thread, where
//! `AppKit` lives, by dispatching onto the main queue. It is serviced by the main run loop, so this
//! needs `freddie_main_loop` running and `NSApp` initialized, the same as the menu bar.
//!
//! macOS only.

use std::cell::RefCell;

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
    // The panel and its label, on the main thread. Only ever touched inside a dispatched block,
    // which always runs on main.
    static PANEL: RefCell<Option<(Retained<NSPanel>, Retained<NSTextField>)>> =
        const { RefCell::new(None) };
}

/// Show the overlay with `text`, from any thread.
///
/// The panel is sized to the text, so a keymap with more rows makes a taller panel rather than a
/// clipped one.
///
/// # Panics
///
/// If the dispatched block somehow runs off the main queue, which would mean libdispatch handed
/// the main queue's work to another thread.
pub fn show(text: &'static str) {
    DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("dispatched to the main queue");
        PANEL.with_borrow_mut(|slot| {
            let (panel, label) = slot.get_or_insert_with(|| build(mtm));
            // SAFETY: setting the label's text, resizing to fit it, and placing the panel, on the
            // main thread.
            {
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
                {
                    panel.orderOut(None);
                }
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
    (panel, label)
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
