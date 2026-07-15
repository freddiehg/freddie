//! A macOS menu-bar status item with a single Quit entry.
//!
//! [`show`] builds the status item and its one-item menu and registers a callback
//! that fires when Quit is chosen. Call it on the main thread, AFTER `NSApp` is
//! initialized (see `freddie_main_loop::init_menu_bar_app`): tray-icon creates an
//! `NSStatusItem`, which macOS requires on the main thread, and the status item
//! needs an app to live in.
//!
//! The returned [`MenuBar`] owns the status item. Hold it for as long as the icon
//! should be visible; dropping it removes the icon. It is `!Send`, so it stays on
//! the main thread that created it.
//!
//! macOS only.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// A live status item. Holding it keeps the icon up; dropping it takes the icon down.
pub struct MenuBar {
    _tray: TrayIcon,
}

/// Shows the menu-bar status item with a single Quit entry. `on_quit` runs, on the
/// main thread, when the user chooses Quit.
///
/// # Errors
///
/// Returns the underlying error if the menu or the status item cannot be created.
pub fn show(
    on_quit: impl Fn() + Send + Sync + 'static,
) -> Result<MenuBar, Box<dyn std::error::Error + Send + Sync>> {
    // The one menu item, and its id so the handler can tell it apart from any future
    // item. `None` is the keyboard accelerator: a status-item menu does not need one.
    let quit = MenuItem::new("Quit", true, None);
    let quit_id = quit.id().clone();

    let menu = Menu::new();
    menu.append(&quit)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(mercury_icon()?)
        // A template image: macOS ignores the RGB and renders the alpha mask in the
        // menu bar's own color (white on a dark bar, black on a light one), so the
        // black glyph shows correctly on either without inverting it.
        .with_icon_as_template(true)
        .with_tooltip("mercury")
        .build()?;

    // muda delivers menu events through one global handler. It fires on the main
    // thread, during menu tracking, which the NSApp pump (freddie_main_loop) drives.
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == quit_id {
            on_quit();
        }
    }));

    Ok(MenuBar { _tray: tray })
}

/// The ☿ status-item icon: the bundled glyph, trimmed to its shape and sized for the
/// menu bar, as a template (alpha mask, recolored by macOS).
///
/// The source PNG is a black glyph on a transparent background at 800x800 with wide
/// margins. Trimming to the glyph's bounds and then scaling makes the glyph fill the
/// bar rather than sit tiny inside the margins; a few pixels of padding keep it off
/// the bar's top and bottom.
fn mercury_icon() -> Result<Icon, Box<dyn std::error::Error + Send + Sync>> {
    let img = image::load_from_memory(include_bytes!("../assets/mercury.png"))?.into_rgba8();
    let glyph = crop_to_alpha(&img);

    // tray-icon renders the icon at 18pt tall; a ~2x pixel height keeps it crisp on a
    // Retina bar. The glyph is portrait, so width follows from its aspect.
    let glyph_h: u32 = 30;
    let glyph_w = (glyph.width() * glyph_h).div_ceil(glyph.height().max(1)).max(1);
    let scaled = image::imageops::resize(
        &glyph,
        glyph_w,
        glyph_h,
        image::imageops::FilterType::Lanczos3,
    );

    let pad: u32 = 4;
    let mut canvas = image::RgbaImage::new(glyph_w + 2 * pad, glyph_h + 2 * pad);
    image::imageops::overlay(&mut canvas, &scaled, i64::from(pad), i64::from(pad));

    let (w, h) = (canvas.width(), canvas.height());
    Ok(Icon::from_rgba(canvas.into_raw(), w, h)?)
}

/// Crops an image to the bounding box of its non-transparent pixels. Returns a clone
/// unchanged if the image is fully transparent.
fn crop_to_alpha(img: &image::RgbaImage) -> image::RgbaImage {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    for (x, y, px) in img.enumerate_pixels() {
        if px.0[3] > 16 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x > max_x {
        return img.clone();
    }
    image::imageops::crop_imm(img, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1).to_image()
}
