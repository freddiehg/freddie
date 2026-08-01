use bind::{Bind, and, if_not_invalidated};
use freddie_keys::Key;
use laserbeam::HasAncestor;

#[allow(clippy::wildcard_imports)]
use crate::handlers::*;
use crate::{MercuryStruct, Site};

use super::{ForegroundedApp, MercuryPath, ReturnHomeLayersPath, SiteLayerPath};

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
#[node(parent_path = ReturnHomeLayersPath)]
#[binds(MercuryStruct)]
#[derived_child(site_data)]
#[bind(
    Key::Escape.down() => if_not_invalidated(go_home),
    Key::KeyT.down() => if_not_invalidated(enter_typing),
)]
pub struct SiteLayer;

impl SiteLayer {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self
    }
}

/// The site's level, which is not in the tree. A site with no bindings is not a variant, and
/// [`site_data`] returns `None` for it.
#[derive(Bind, Debug)]
#[derived_node(parent_path = SiteLayerPath)]
#[binds(MercuryStruct)]
pub enum SiteData {
    ClaudeAi(ClaudeAiSite),
}

/// Reads the front tab's URL, the only copy, and builds the level for the site it names.
///
/// `None` whenever Chrome is not the confirmed front app, whenever the tab source has not reported
/// yet, and for a site with no bindings. The first two are the same "we do not know" that leaves a
/// key unbound rather than aimed at whatever site was there before.
fn site_data<'a, P: HasAncestor<MercuryPath<'a>>>(path: &P) -> Option<SiteData> {
    let root = path.ancestor();
    let url = root
        .foreground
        .as_ref()
        .and_then(ForegroundedApp::chrome)?
        .url
        .as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite)),
        Site::Other => None,
    }
}

/// claude.ai's level, where `n` starts a new chat.
#[derive(Bind, Debug)]
#[derived_node(parent_path = SiteLayerPath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyN.down() => if_not_invalidated(and!(tap_cmd_shift_o, enter_typing)))]
pub struct ClaudeAiSite;
