//! The event sources: a keyboard, and the OS reporting a newly foregrounded app.

use bind::EventTrigger;
use freddie_keys::KeyEvent;
use freddie_windows::WindowChange;

// A specific key is its own trigger: `Key::KeyR` binds that key. The type and its
// `EventTrigger` impl live in `freddie_keys`, so no wrapper is needed here.

/// A keyboard trigger matching every key, modifier or not, on either press.
///
/// Bound at the root as the last resort (dispatch is leafward, so a key the active layer binds
/// wins). A key no layer claimed falls here, and `maybe_pass_through` splits modifier from
/// non-modifier: a modifier is recorded in `held`, and either is passed through in a passthrough
/// layer or swallowed otherwise. Caps lock, not a modifier, rides this like any other key.
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

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
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

/// A trigger that matches any window change, whichever kind it is.
///
/// One binding at the root answers all of them, the way [`Foregrounded`] does for app
/// activation: every variant records into the same place, and nothing branches on the kind
/// until it gets there.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Windowed;

/// The window source reported a change.
#[cfg_attr(feature = "testing", derive(PartialEq))]
#[derive(Debug)]
pub struct WindowEvent {
    pub change: WindowChange,
}
impl EventTrigger for Windowed {
    type Event = WindowEvent;
    fn is_matching(&self, _ev: &WindowEvent) -> bool {
        true
    }
}

/// A trigger that matches any tab-reported event, whichever URL it carries.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Tabbed;

/// The browser reported the front tab's URL.
///
/// From the extension over the event socket, never from the OS: the active tab is Chrome's to know
/// and no app-activation event carries it. Pushed on every tab switch and navigation, so mercury
/// never asks and never polls.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct TabEvent {
    pub url: String,
}
impl EventTrigger for Tabbed {
    type Event = TabEvent;
    fn is_matching(&self, _ev: &TabEvent) -> bool {
        true
    }
}

/// A quit request, wherever it came from (the menu bar for now). It carries no key: it is a
/// single, layer-independent "quit now", so one type is both the trigger and the event.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quit;

bind::self_trigger!(Quit);

/// The apps Mercury knows about. `Other` is anything it has no bindings for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum App {
    Chrome,
    Finder,
    Ghostty,
    Zed,
    #[default]
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

/// The site a tab belongs to. `Other` is anything mercury has no bindings for, exactly as with
/// [`App`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Site {
    ClaudeAi,
    Other,
}

impl Site {
    /// Which site `url` belongs to. The browser-tab analog of [`App::from_bundle_id`].
    ///
    /// The host has to match exactly, so `claude.ai.evil.com` is [`Site::Other`]: a suffix match
    /// would hand any domain that ends the right way whatever binds the real site has.
    #[must_use]
    pub fn from_url(url: &str) -> Self {
        match site_host(url) {
            Some("claude.ai") => Self::ClaudeAi,
            _ => Self::Other,
        }
    }
}

/// The host that identifies a site: [`host`] without a leading `www.`, because `www.claude.ai` and
/// `claude.ai` are the same site and binding one of them only would be a coin flip.
fn site_host(url: &str) -> Option<&str> {
    host(url).map(|host| host.strip_prefix("www.").unwrap_or(host))
}

/// The host of `url` as it appears, without a port or userinfo: `https://www.x.com/a` is
/// `www.x.com`. `None` for anything with no host at all, which is `about:blank` and `file:///...`.
///
/// The `www.` is kept because this is also what a copy puts on the clipboard, and a URL's host is
/// what it says it is. [`site_host`] is the one that drops it, for matching.
///
/// Chrome hands up a URL it has already normalized, so the host arrives lowercased and there is no
/// case folding to do here. Hand-rolled rather than the `url` crate, whose idna support pulls the
/// ICU4X tree for a comparison this covers.
#[must_use]
pub fn host(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let authority = after_scheme
        .find(['/', '?', '#'])
        .map_or(after_scheme, |end| &after_scheme[..end]);
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_port
        .find(':')
        .map_or(host_port, |end| &host_port[..end]);
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::{Site, host, site_host};

    // The host as it appears, `www.` and all: this is what a copy puts on the clipboard.
    #[test]
    fn the_host_is_the_one_the_url_carries() {
        for (url, want) in [
            ("https://claude.ai/new", Some("claude.ai")),
            ("https://claude.ai", Some("claude.ai")),
            ("https://claude.ai?q=1", Some("claude.ai")),
            ("https://claude.ai#top", Some("claude.ai")),
            ("https://www.x.com/asdfasdf", Some("www.x.com")),
            ("http://claude.ai:8080/x", Some("claude.ai")),
            ("https://user:pw@claude.ai/x", Some("claude.ai")),
            ("https://claude.ai.evil.com/", Some("claude.ai.evil.com")),
            ("https://notclaude.ai/", Some("notclaude.ai")),
            ("chrome://extensions", Some("extensions")),
            ("about:blank", None),
            ("file:///Users/x", None),
            ("", None),
        ] {
            assert_eq!(host(url), want, "{url}");
        }
    }

    // The host a site is matched on drops the `www.`, and only a leading one.
    #[test]
    fn the_site_host_drops_the_www() {
        for (url, want) in [
            ("https://www.claude.ai/x", Some("claude.ai")),
            ("https://claude.ai/x", Some("claude.ai")),
            ("https://www.x.com/asdfasdf", Some("x.com")),
            ("https://notwww.claude.ai/", Some("notwww.claude.ai")),
            ("about:blank", None),
        ] {
            assert_eq!(site_host(url), want, "{url}");
        }
    }

    #[test]
    fn only_the_exact_host_is_the_site() {
        for (url, want) in [
            ("https://claude.ai/new", Site::ClaudeAi),
            ("https://www.claude.ai/", Site::ClaudeAi),
            ("https://claude.ai.evil.com/", Site::Other),
            ("https://evil.com/claude.ai", Site::Other),
            ("about:blank", Site::Other),
        ] {
            assert_eq!(Site::from_url(url), want, "{url}");
        }
    }
}
