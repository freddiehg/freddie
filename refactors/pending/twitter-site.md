# x.com in the site layer

Not built. The second site after `claude.ai`, and the one that tests whether the site layer generalizes or was shaped around a single case.

x.com already has a keyboard interface, and the first version of this used it: bind `f` to a tap of `l` and let x.com do the liking. That is a veneer, and it breaks when x.com renames a shortcut. What is here instead is a selection the extension owns, with mercury sending what to do to it, which is the shape that works on a site with no shortcuts at all.

## The keymap

Every key here is a command to the extension, except `h`, which is the browser's back, and `t` and `esc`, which are the layer's own.

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

## The extension owns the selection

The binds above are a veneer over x.com's own shortcuts: `f` taps `l` and hopes x.com still means "like" by it. That works until x.com changes its shortcut set, and it can never work on a site that has no shortcuts at all. So the selection is the extension's, and mercury sends intent rather than keystrokes.

That is the difference between supporting x.com and supporting lists. The same content script shape, pointed at a different selector, gives `j` and `k` to any site with a list of things.

### It lives in the content script

Three places it could go, and two are ruled out by things already decided.

Not in the level. `XSite` is a derived child, rebuilt from root state every dispatch, and `refactors/past/derived-child-persistence.md` rejects giving one a longer life: persisting means storing, storing means it is in the tree, and in the tree it can disagree with what it was derived from.

Not in mercury at all. A `selected` at the root that mercury increments on `j` is a shadow of something in a page it cannot see, wrong the moment you scroll, click, or x.com prepends new posts, with no event that reconciles it.

Not in the service worker either, which is the part that matters for the extension as built: it is killed after roughly 30s idle, so anything it remembers is gone by the time you come back to the tab. A content script lives as long as the page does, which is exactly the lifetime the selection has.

### Keyed by the post, not the index

The timeline mutates under you. x.com prepends new posts, infinite-scrolls more onto the end, and removes some as you go. An index into a list that grows at the front points at a different post every time that happens.

So the content script stores the focused post's own id, read out of its permalink, and resolves it to an element when it needs one:

```ts
type Selection = { statusId: string } | null;

let selection: Selection = null;

function items(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('article[data-testid="tweet"]')];
}

function move(delta: number): void {
  const list = items();
  const current = list.findIndex((el) => statusIdOf(el) === selection?.statusId);
  // No selection yet, or the selected post is gone from the DOM: start from the top.
  const next = list[current === -1 ? 0 : Math.min(Math.max(current + delta, 0), list.length - 1)];
  if (next === undefined) return;
  selection = { statusId: statusIdOf(next) ?? "" };
  highlight(next);
  next.scrollIntoView({ block: "nearest" });
}
```

A selected post scrolled out of the DOM entirely is the case that has to be handled rather than crashed on, and starting from the top is the honest answer: the thing being pointed at is gone.

The highlight is the extension's too, an outline on the selected element. Without it the selection is invisible and `j` looks broken, which is not a thing x.com's own focus ring can be relied on to do once mercury stops using x.com's own navigation.

### What mercury sends

The binds stop being taps and become commands, which makes this doc depend on `external-effects.md` landing rather than on nothing:

```rust
pub enum Command {
    // …
    /// Move the selection in the page's list, by one or by a screen.
    #[serde(rename = "Command.SelectMove")]
    SelectMove(SelectMove),
    /// Do something to the selected item. The extension decides what that means for its site.
    #[serde(rename = "Command.SelectAct")]
    SelectAct(SelectAct),
    /// The selected item's canonical URL, for yanking or opening.
    #[serde(rename = "Command.SelectUrl")]
    SelectUrl,
}

pub struct SelectMove {
    pub delta: i32,
}

pub struct SelectAct {
    pub action: SelectAction,
}

/// What the extension is being asked to do to the selected item. Named actions rather than a
/// selector or a script, for the reason `RunJs` is opt-in: each one is a thing with a meaning, and
/// a site that cannot do one says so.
pub enum SelectAction {
    Open,
    Like,
    Reply,
    Repost,
    Bookmark,
}
```

`SelectAction` is the vocabulary, and each site's content script maps it onto that site's buttons. x.com's `Like` clicks `[data-testid="like"]` on the selected article. A site that has no such thing replies `Result.Err` and the bind does nothing, which is what a bind for a thing that is not there should do.

`h`, going back, stays a plain `cmd-[` tap: it is the browser's, not the list's.

### The service worker in the middle

The content script has the state and the socket is the service worker's, so a command crosses `chrome.tabs.sendMessage` on the way in and its reply comes back the same way. The worker holds no selection and needs none, which is what keeps its 30s death irrelevant.

A tab with no content script running, because the page loaded before the extension did, replies with nothing. That is an `Err` to mercury and a no-op for the bind, and reloading the tab is the fix.

## What is not decided

x.com is one host with several kinds of page, and the binds above are the ones that make sense on all of them. On a single post's page there is one post rather than a list, so `j` and `k` mean less and "open the quoted post" or "open the author" would mean something they cannot mean on a timeline.

That is the path-matching question, deferred when `Site::from_url` shipped host-only, and this is the site that raises it. Three shapes, none picked here:

- Leave it host-only. One level for x.com, binds that work everywhere, no per-page binds offered.
- A derived child under the site's level, keyed on the path. The chain is three deep already and this makes it four.
- `Site::from_url` returns the page with the site, `Site::X(XPage::Post)` against `XPage::Timeline`, so one level matches internally and the tree does not grow. The smallest change, at the cost of parsing a path on every dispatch, which is what host-only matching was avoiding.

Deciding it is blocked on something else first. x.com is a single-page application, so moving from the timeline into a post is a History API navigation, and `chrome.tabs.onUpdated` is not the event for those: the site layer would keep whatever URL it was last told. Per-page binds need `chrome.webNavigation.onHistoryStateUpdated` in the extension, which is the case `refactors/past/chrome-extension.md` records as costing nothing while matching stays host-only. It stops being free here.
