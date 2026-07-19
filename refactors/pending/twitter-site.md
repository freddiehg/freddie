# x.com in the site layer

Not built. The second site after `claude.ai`, and the one that tests whether the site layer generalizes or was shaped around a single case.

x.com already has a keyboard interface, so the job is not to give it one. It is to put a vim-shaped one over the top: `h` `j` `k` `l` move and enter, and the actions sit under letters that mean the action rather than under whatever x.com picked.

## The keymap

`j` and `k` are x.com's own, so they pass through as themselves. Everything else is a remap onto a key x.com already understands, which keeps this a remap rather than an automation, and keeps it working with nothing from `external-effects.md`.

```
  X.COM
  ────────────────────
  j    next post
  k    previous post
  l    open post
  h    back
  f    like
  r    reply
  e    repost
  b    bookmark
  t    typing
  esc  home
```

`l` opens the focused post, which is `enter` to x.com. `h` goes back, which is the browser's back rather than anything x.com binds. Together they are the pair the sketch asked for: `l` descends into a post, `h` returns to where you were.

Actions keep their meanings and give up x.com's letters. `f` for like, because `l` is taken by navigation and favourite is the older name for the thing. `e` for repost, because x.com's `t` is mercury's typing key in every layer that binds keys as commands, and losing the way to typing on one site would be worse than moving one action.

## Confirm before building

This is written from x.com's shortcut help (`?`) and its shortcut set drifts. Check each one against the running site:

- `j` and `k` move between posts, `enter` opens the focused one.
- `l` likes, `r` replies, `t` reposts, `b` bookmarks.
- The `g` sequences (`g h` home, `g n` notifications, `g p` profile, and the rest).

Also check that x.com's shortcuts do anything at all when no post is focused, because `j` is what focuses the first one, and a bind that only works after another bind is a thing the overlay should say.

## The binds

```rust
/// x.com's level. Vim motions over x.com's own keys, and actions under letters that name the
/// action rather than under the ones x.com happened to pick.
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyJ.down() => next_post,
    Key::KeyK.down() => previous_post,
    Key::KeyL.down() => open_post,
    Key::KeyH.down() => back,
    Key::KeyF.down() => like,
    Key::KeyR.down() => reply,
    Key::KeyE.down() => repost,
    Key::KeyB.down() => bookmark,
)]
pub struct XSite {}
```

Each handler is one tap of the key x.com is waiting for:

```rust
pub(crate) fn like<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    vec![tap(Key::KeyL, ModifierFlags::empty())]
}

/// The browser's back, not x.com's: `cmd-[` is what Chrome takes.
pub(crate) fn back<E, N>(_ev: &E, _node: N) -> Vec<MercuryEffect> {
    vec![tap(Key::BracketLeft, ModifierFlags::COMMAND)]
}
```

`j` and `k` are the odd ones: they remap to themselves. They are bound anyway rather than left to fall through, because the site layer swallows what it does not bind, and a layer where `j` silently does nothing while `k` works would be worse than one redundant tap.

## Matching the host

Two names for one site, which is the first of those:

```rust
 pub enum Site {
     ClaudeAi,
+    X,
     Other,
 }

 pub fn from_url(url: &str) -> Self {
     match host(url) {
         Some("claude.ai") => Self::ClaudeAi,
+        Some("x.com" | "twitter.com") => Self::X,
         _ => Self::Other,
     }
 }
```

`twitter.com` still resolves and redirects, and `host` already strips `www.`.

## Yanking the URL is not this

Copying the front tab's URL is universal: every page has one and nothing about it is x.com's. It belongs in the Chrome in-app layer, beside refresh, and it is worth writing here because it is the bind that made the distinction obvious.

It also needs nothing from the browser. mercury already holds the URL, in `ForegroundedChrome.url`, put there by the extension. So the handler reads the state it already has:

```rust
/// `y` in Chrome: copy the front tab's URL.
pub(crate) fn yank_url(_ev: &KeyEvent, node: /* ChromeApp node */) -> Vec<MercuryEffect> {
    let Some(url) = /* ascend to root */.foreground.confirmed_chrome().and_then(|c| c.url.clone())
    else {
        return Vec::new();
    };
    vec![MercuryEffect::Copy(url)]
}
```

What it needs is a clipboard effect, which mercury does not have:

```rust
 pub enum MercuryEffect {
     Foreground(App),
     Tap(Chord),
     // …
+    /// Put text on the system clipboard.
+    Copy(String),
 }
```

Performed with `NSPasteboard`, which is AppKit and so main-thread-bound, the same constraint `freddie_windows` and the menu bar already work under. That makes it a `freddie_clipboard` crate rather than something mercury does inline, because figaro would write it identically.

Yanking a single post's URL, rather than the page's, is a different thing: it is per-post, x.com has no shortcut for it, and it needs the browser asked. That one is `external-effects.md` and is not in this doc.

## What is not decided

x.com is one host with several kinds of page, and the binds above are the ones that make sense on all of them. On a single post's page there is one post rather than a list, so `j` and `k` mean less and "open the quoted post" or "open the author" would mean something they cannot mean on a timeline.

That is the path-matching question, deferred when `Site::from_url` shipped host-only, and this is the site that raises it. Three shapes, none picked here:

- Leave it host-only. One level for x.com, binds that work everywhere, no per-page binds offered.
- A derived child under the site's level, keyed on the path. The chain is three deep already and this makes it four.
- `Site::from_url` returns the page with the site, `Site::X(XPage::Post)` against `XPage::Timeline`, so one level matches internally and the tree does not grow. The smallest change, at the cost of parsing a path on every dispatch, which is what host-only matching was avoiding.

Deciding it is blocked on something else first. x.com is a single-page application, so moving from the timeline into a post is a History API navigation, and `chrome.tabs.onUpdated` is not the event for those: the site layer would keep whatever URL it was last told. Per-page binds need `chrome.webNavigation.onHistoryStateUpdated` in the extension, which is the case `refactors/past/chrome-extension.md` records as costing nothing while matching stays host-only. It stops being free here.
