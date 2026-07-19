use bind::Bind;
use freddie::TimerGuard;
use freddie_keys::Key;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{MercuryEffect, MercuryStruct, Site};

use super::{LayerPath, SiteLayerPath, arm_return_home};

pub(crate) const OVERLAY: &str = include_str!("overlays/site.txt");
pub(crate) const CLAUDE_AI_OVERLAY: &str = include_str!("overlays/claude-ai.txt");

/// The keymap the overlay shows for the site layer, given the site in the front tab.
pub(crate) const fn overlay_for(site: Option<Site>) -> &'static str {
    match site {
        Some(Site::ClaudeAi) => CLAUDE_AI_OVERLAY,
        Some(Site::Other) | None => OVERLAY,
    }
}

/// The per-tab layer, `u` from home.
///
/// Separate from the in-app layer on purpose. In-app is what Chrome the application can do, and it
/// holds whatever is true of every tab; this holds what the site in the front tab can do, which
/// changes as you move between tabs without the frontmost app changing at all.
///
/// It stores no site: [`site_data`] reads the front tab's URL from the root on every dispatch, so
/// switching tabs while sitting in this layer changes what is bound with no event of its own.
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
#[bind(
    |path| path.get().home_timeout.trigger() => to_home,
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
)]
pub struct SiteLayer {
    // Read for the trigger matching its firing, and held for its `Drop`: dropping the guard
    // cancels this layer's return-home timer.
    pub(crate) home_timeout: TimerGuard,
}

impl SiteLayer {
    /// Build the site layer with its return-home timer armed, returning the layer and the effect
    /// that schedules it.
    #[must_use]
    pub(crate) fn new() -> (Self, MercuryEffect) {
        let (timeout, timer) = arm_return_home();
        (
            Self {
                home_timeout: timeout,
            },
            timer,
        )
    }

    /// Reset the return-home timer on activity in this layer: drop the old guard (cancelling it)
    /// and arm a fresh one, returning the effect that schedules it.
    #[must_use]
    pub(crate) fn rearm(&mut self) -> MercuryEffect {
        let (timeout, timer) = arm_return_home();
        self.home_timeout = timeout;
        timer
    }
}

/// The site's level, which is not in the tree. A site with no bindings is not a variant, and
/// [`site_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
pub enum SiteData {
    ClaudeAi(ClaudeAiSite),
}

/// Reads the front tab's URL, the only copy, and builds the level for the site it names.
///
/// `None` whenever Chrome is not the confirmed front app, whenever the tab source has not reported
/// yet, and for a site with no bindings. The first two are the same "we do not know" that leaves a
/// key unbound rather than aimed at whatever site was there before.
fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    // SiteLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite {})),
        Site::Other => None,
    }
}

/// claude.ai's level, where `n` starts a new chat.
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyN.down() => new_chat)]
pub struct ClaudeAiSite {}
