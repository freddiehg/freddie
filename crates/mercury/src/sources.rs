//! The event sources: a keyboard, and the OS reporting a newly foregrounded app.

use bind::EventTrigger;
use freddie_keys::KeyEvent;

// A specific key is its own trigger: `Key::KeyR` binds that key. The type and its
// `EventTrigger` impl live in `freddie_keys`, so no wrapper is needed here.

/// A keyboard trigger that matches every key, on either press.
///
/// A catch-all: when a layer binds it, it shadows an ancestor's binding for the same key
/// (dispatch is leafward). There is no ordering between it and a specific-key trigger, so
/// binding both on one active path is a shadow, not a conflict.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnyKey;
impl EventTrigger for AnyKey {
    type Event = KeyEvent;
    fn is_matching(&self, _ev: &KeyEvent) -> bool {
        true
    }
}

/// A trigger that matches any app-foregrounded event, whichever app it is.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Foregrounded;

/// A fired app-foregrounded event.
#[derive(Debug)]
pub struct ForegroundEvent {
    pub app: App,
}
impl EventTrigger for Foregrounded {
    type Event = ForegroundEvent;
    fn is_matching(&self, _ev: &ForegroundEvent) -> bool {
        true
    }
}

/// A trigger that matches a quit request, wherever it came from (the menu bar for
/// now). It carries no key: it is a single, layer-independent "quit now".
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quit;

/// A fired quit request.
#[derive(Debug)]
pub struct QuitEvent;

impl EventTrigger for Quit {
    type Event = QuitEvent;
    fn is_matching(&self, _ev: &QuitEvent) -> bool {
        true
    }
}

/// The apps Mercury knows about. `Other` is anything it has no bindings for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum App {
    Chrome,
    Finder,
    Ghostty,
    Zed,
    Other,
}

impl App {
    /// Maps a bundle identifier, as `freddie_app_nav` reports it, to a known app. Anything
    /// unrecognized is [`App::Other`].
    ///
    /// This is the consumer's half of the app-nav contract: the watcher hands up a string and
    /// Mercury decides which of its apps it is. Bundle ids are the stable name for an app,
    /// unlike display names, which differ depending on who is asked (`System Events` says
    /// `ghostty`, the app says `Ghostty`).
    #[must_use]
    pub fn from_bundle_id(bundle_id: &str) -> Self {
        match bundle_id {
            "com.google.Chrome" => Self::Chrome,
            "com.apple.finder" => Self::Finder,
            "com.mitchellh.ghostty" => Self::Ghostty,
            "dev.zed.Zed" => Self::Zed,
            _ => Self::Other,
        }
    }

    /// The bundle identifier to hand `freddie_app_nav::foreground` to bring this app up. It is
    /// the same string [`from_bundle_id`](Self::from_bundle_id) matches, so the two round-trip.
    /// [`App::Other`] is not a specific app, so it has none.
    #[must_use]
    pub const fn bundle_id(self) -> Option<&'static str> {
        match self {
            Self::Chrome => Some("com.google.Chrome"),
            Self::Finder => Some("com.apple.finder"),
            Self::Ghostty => Some("com.mitchellh.ghostty"),
            Self::Zed => Some("dev.zed.Zed"),
            Self::Other => None,
        }
    }
}
