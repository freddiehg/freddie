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
use tray_icon::{TrayIcon, TrayIconBuilder};

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

    // A title rather than an icon for v0: text in the menu bar needs no image asset.
    // `\u{263F}` is ☿, the mercury symbol.
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("\u{263F}")
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
