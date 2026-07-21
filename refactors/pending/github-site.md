# github's levels

The site layer holds what the site in the front tab can do. Today a site is a host and nothing more, so `github.com` is one thing whether the tab is showing the dashboard, a repository, or a pull request, and a key bound for github.com is bound on all three.

What we want is bound to where you are:

- On a repository page, `c` clones it into `~/code/<repo>`.
- On a pull request page, `c` checks that pull request out in `~/code/<repo>`, by its number.
- On any github.com page, whatever is true of the whole site is bound, and the page you are on adds to it rather than replacing it.
- The overlay shows the keymap of the page you are on, so `o` on a pull request lists the pull request's keys.

The site's level gets a level of its own beneath it: `SiteLayer` derives `GithubSite` from the URL, and `GithubSite` derives `GithubRepoSite` or `GithubPullRequestSite` from the same URL. Both are derived, so nothing is stored and nothing goes stale; navigating from a repository to one of its pull requests changes what is bound with no event beyond the URL the extension already reports.

The state each level carries is what the URL says: an owner, a repository name, a pull request number. Nothing is looked up, and nothing that the URL does not carry — a branch name, a base ref, a title — is in the model. `gh pr checkout 123` resolves the branch itself, in the clone, at the moment it runs.

Depends on `run-effect.md` for `MercuryEffect::Run`, and on `spa-navigation.md`: GitHub navigates between these routes without loading a document, so without it mercury holds whichever route the tab landed on.

## Change 1: a URL has a path

`host` stops at the authority. The routes need what comes after it.

`crates/mercury/src/sources.rs`, next to `host`:

```rust
/// The path of `url`, without the query or the fragment: `https://github.com/a/b?tab=x` is `/a/b`.
///
/// Empty for a URL that names only a host, so a caller that splits on `/` sees no segments either
/// way and `https://github.com` and `https://github.com/` are the same route.
#[must_use]
pub fn path(url: &str) -> &str {
    let Some(after_scheme) = url.split_once("://").map(|(_, rest)| rest) else {
        return "";
    };
    let Some(start) = after_scheme.find('/') else {
        return "";
    };
    let path = &after_scheme[start..];
    path.find(['?', '#']).map_or(path, |end| &path[..end])
}
```

with the test table beside the existing ones:

```rust
#[test]
fn the_path_is_what_follows_the_authority() {
    for (url, want) in [
        ("https://github.com/a/b", "/a/b"),
        ("https://github.com/a/b/", "/a/b/"),
        ("https://github.com/a/b?tab=readme", "/a/b"),
        ("https://github.com/a/b#L4", "/a/b"),
        ("https://github.com", ""),
        ("https://github.com/", "/"),
        ("https://user:pw@github.com/a", "/a"),
        ("about:blank", ""),
        ("", ""),
    ] {
        assert_eq!(path(url), want, "{url}");
    }
}
```

## Change 2: a site carries its route

`Site` becomes an enum whose github variant says where in github.com the URL points. It stops being `Copy`, because a route carries an owner and a name.

`crates/mercury/src/sources.rs`, before:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Site {
    ClaudeAi,
    Other,
}

impl Site {
    #[must_use]
    pub fn from_url(url: &str) -> Self {
        match site_host(url) {
            Some("claude.ai") => Self::ClaudeAi,
            _ => Self::Other,
        }
    }
}
```

after:

```rust
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Site {
    ClaudeAi,
    Github(GithubRoute),
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
            Some("github.com") => Self::Github(GithubRoute::from_path(path(url))),
            _ => Self::Other,
        }
    }
}

/// A repository, as its URL names it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

/// Where in github.com a URL points, to the depth anything binds a key to.
///
/// Everything under a repository that is not a pull request is [`Repo`](Self::Repo): the file
/// browser, the issues list, and the settings of a repository are all pages where "clone this"
/// means the same thing. A pull request is its own route because it has a key of its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GithubRoute {
    /// github.com naming no repository: the dashboard, a profile, the site's own pages.
    Root,
    Repo(Repo),
    PullRequest(PullRequest),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequest {
    pub repo: Repo,
    pub number: u32,
}

/// github.com's own first path segments, which are pages rather than owners.
///
/// Without this, `github.com/settings/keys` is a repository called `keys` owned by `settings`, and
/// `c` on it offers to clone something that does not exist. The list is what github.com actually
/// serves at the top level; a segment missing from it is treated as an owner, which is the safe
/// direction: the worst case is a clone that fails and says so in the log.
const GITHUB_RESERVED: &[&str] = &[
    "about", "account", "apps", "codespaces", "collections", "contact", "dashboard", "enterprise",
    "events", "explore", "features", "issues", "join", "login", "logout", "marketplace", "new",
    "notifications", "organizations", "orgs", "pricing", "pulls", "search", "security",
    "sessions", "settings", "sponsors", "stars", "topics", "trending",
];

impl GithubRoute {
    /// The route `path` names, `path` being [`path`]'s output for a github.com URL.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        let (Some(owner), Some(name)) = (segments.next(), segments.next()) else {
            return Self::Root;
        };
        if GITHUB_RESERVED.contains(&owner) {
            return Self::Root;
        }
        let repo = Repo {
            owner: owner.to_owned(),
            name: name.to_owned(),
        };
        match (segments.next(), segments.next()) {
            (Some("pull"), Some(number)) => number.parse().map_or(Self::Repo(repo.clone()), |number| {
                Self::PullRequest(PullRequest { repo, number })
            }),
            _ => Self::Repo(repo),
        }
    }
}
```

`number.parse()` rejects `github.com/o/r/pull/new`, which is the compare page and not a pull request, and it rejects anything else that is not a number. Those fall back to the repository, which is what the page is.

The tests:

```rust
#[test]
fn a_github_url_names_its_route() {
    let freddie = || Repo {
        owner: "rbalicki2".to_owned(),
        name: "freddie".to_owned(),
    };
    for (url, want) in [
        ("https://github.com", GithubRoute::Root),
        ("https://github.com/", GithubRoute::Root),
        ("https://github.com/rbalicki2", GithubRoute::Root),
        ("https://github.com/settings/keys", GithubRoute::Root),
        ("https://github.com/pulls", GithubRoute::Root),
        ("https://github.com/rbalicki2/freddie", GithubRoute::Repo(freddie())),
        ("https://github.com/rbalicki2/freddie/", GithubRoute::Repo(freddie())),
        ("https://github.com/rbalicki2/freddie/tree/master/crates", GithubRoute::Repo(freddie())),
        ("https://github.com/rbalicki2/freddie/pulls", GithubRoute::Repo(freddie())),
        ("https://github.com/rbalicki2/freddie/pull/new/branch", GithubRoute::Repo(freddie())),
        (
            "https://github.com/rbalicki2/freddie/pull/12",
            GithubRoute::PullRequest(PullRequest { repo: freddie(), number: 12 }),
        ),
        (
            "https://github.com/rbalicki2/freddie/pull/12/files",
            GithubRoute::PullRequest(PullRequest { repo: freddie(), number: 12 }),
        ),
    ] {
        assert_eq!(Site::from_url(url), Site::Github(want), "{url}");
    }
}
```

`Site` losing `Copy` reaches one place, `Layer::overlay_content`, which Change 5 rewrites anyway.

## Change 3: the model knows where clones go

A handler cannot read `HOME`, and the effect side cannot fill one in. So the directory clones go into is a fact about the outside world that arrives the way every other one does: read while the model is being built, and held in state after.

`crates/mercury/src/state/mod.rs`, before:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub windows: Windows,
    pub typing_state: TypingState,
    overlay: Option<TimerGuard>,
    #[resolve_into]
    layer: Layer,
}
```

after:

```rust
pub struct Mercury {
    pub foreground: Foreground,
    pub windows: Windows,
    pub typing_state: TypingState,
    /// Where a checkout goes: `~/code`, absolute, read once at construction.
    ///
    /// In the model because a handler that asks for `git clone` has to say where, and reading the
    /// environment in a handler is exactly what a handler does not do.
    pub code_dir: PathBuf,
    overlay: Option<TimerGuard>,
    #[resolve_into]
    layer: Layer,
}
```

`Mercury::new`, before:

```rust
    pub fn new(front_app: App, windows: Windows) -> Self {
```

after:

```rust
    pub fn new(front_app: App, windows: Windows, code_dir: PathBuf) -> Self {
```

with `code_dir` stored alongside the other fields.

`crates/mercury/src/daemon.rs`, `Boot`, before:

```rust
struct Boot {
    front_app: App,
    windows: Windows,
    window_sink: Option<WindowSink>,
}
```

after:

```rust
struct Boot {
    front_app: App,
    windows: Windows,
    window_sink: Option<WindowSink>,
    code_dir: PathBuf,
}
```

filled where the rest of `Boot` is:

```rust
/// Where clones go. `$HOME/code`, and `/code` if there is no `$HOME`, which is a path a clone
/// will fail on loudly rather than one that quietly lands somewhere unexpected.
fn code_dir() -> PathBuf {
    let mut dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    dir.push("code");
    dir
}
```

and handed to `Mercury::new` at the one call site. The transition tests construct with a fixed `/Users/test/code`, so what they assert is a whole path and not a placeholder.

## Change 4: the levels

`crates/mercury/src/state/site.rs`, before:

```rust
pub enum SiteData {
    ClaudeAi(ClaudeAiSite),
}

fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    // SiteLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite)),
        Site::Other => None,
    }
}
```

after:

```rust
pub enum SiteData {
    ClaudeAi(ClaudeAiSite),
    Github(GithubSite),
}

fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    // SiteLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    let url = root.foreground.confirmed_chrome()?.url.as_deref()?;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite)),
        Site::Github(route) => Some(SiteData::Github(GithubSite { route })),
        Site::Other => None,
    }
}
```

and the github levels, in a `crates/mercury/src/state/github.rs` of their own, because one file per site level is how the split stays legible as sites are added:

```rust
/// github.com's level. It binds what is true of the whole site, and derives the level of the page
/// the tab is on beneath it.
///
/// It holds the route rather than the URL, so the URL is parsed once, in [`site_data`], and the
/// level below reads a route that has already been decided.
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
#[derived_child(github_data)]
pub struct GithubSite {
    pub(crate) route: GithubRoute,
}

/// `GithubSite`'s node, named so the levels below it can say `parent = GithubSitePath`.
pub type GithubSitePath<'a> = bind::Node<SiteLayerPath<'a>, GithubSite>;

/// The page's level. A route with no bindings of its own is not a variant, so the dashboard and a
/// profile page get github.com's binds and nothing more.
#[derive(Bind, Debug)]
#[derived_node(parent = GithubSitePath)]
#[binds(MercuryStruct)]
pub enum GithubData {
    Repo(GithubRepoSite),
    PullRequest(GithubPullRequestSite),
}

/// Reads the route the level above already parsed and builds the page's level from it.
fn github_data(node: &GithubSitePath) -> Option<GithubData> {
    match &node.data.route {
        GithubRoute::Root => None,
        GithubRoute::Repo(repo) => Some(GithubData::Repo(GithubRepoSite { repo: repo.clone() })),
        GithubRoute::PullRequest(pr) => Some(GithubData::PullRequest(GithubPullRequestSite {
            repo: pr.repo.clone(),
            number: pr.number,
        })),
    }
}

/// A repository page, where `c` clones it.
#[derive(Bind, Debug)]
#[derived_node(parent = GithubSitePath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyC.down() => clone_repo)]
pub struct GithubRepoSite {
    pub(crate) repo: Repo,
}

/// A pull request page, where `c` checks it out.
///
/// `c` is the same key as a repository's, because it means the same thing on both: bring what this
/// page is showing onto this machine. On a repository that is the repository; on a pull request the
/// repository is already here and the pull request is what is missing.
#[derive(Bind, Debug)]
#[derived_node(parent = GithubSitePath)]
#[binds(MercuryStruct)]
#[bind(Key::KeyC.down() => checkout_pull_request)]
pub struct GithubPullRequestSite {
    pub(crate) repo: Repo,
    pub(crate) number: u32,
}
```

The route is cloned once per dispatch while the site layer is active and the tab is on github.com: two short strings. The alternative is a level that borrows from the state it was derived from, which the derive cannot express, and the clone buys the levels their own data rather than a route enum every handler has to re-match.

`GithubSite` binds nothing yet. Dispatch is leafward, so anything added to it is bound on a repository page and a pull request page as well as the dashboard, which is the point of it being the parent.

## Change 5: overlays follow the page

`overlay_for` is already a pure function of the site in the front tab. With the route in `Site` it stays one, and the match being exhaustive is what stops a new route from silently showing the wrong keymap.

`crates/mercury/src/state/site.rs`, before:

```rust
pub(crate) const OVERLAY: &str = include_str!("overlays/site.txt");
pub(crate) const CLAUDE_AI_OVERLAY: &str = include_str!("overlays/claude-ai.txt");

/// The keymap the overlay shows for the site layer, given the site in the front tab.
pub(crate) const fn overlay_for(site: Option<Site>) -> &'static str {
    match site {
        Some(Site::ClaudeAi) => CLAUDE_AI_OVERLAY,
        Some(Site::Other) | None => OVERLAY,
    }
}
```

after:

```rust
pub(crate) const OVERLAY: &str = include_str!("overlays/site.txt");
pub(crate) const CLAUDE_AI_OVERLAY: &str = include_str!("overlays/claude-ai.txt");
pub(crate) const GITHUB_OVERLAY: &str = include_str!("overlays/github.txt");
pub(crate) const GITHUB_REPO_OVERLAY: &str = include_str!("overlays/github-repo.txt");
pub(crate) const GITHUB_PR_OVERLAY: &str = include_str!("overlays/github-pull-request.txt");

/// The keymap the overlay shows for the site layer, given the site in the front tab.
///
/// One arm per level that binds a key, so the overlay and the levels are added together: a route
/// that gains a level and not an arm here does not compile.
pub(crate) const fn overlay_for(site: Option<&Site>) -> &'static str {
    match site {
        Some(Site::ClaudeAi) => CLAUDE_AI_OVERLAY,
        Some(Site::Github(GithubRoute::Root)) => GITHUB_OVERLAY,
        Some(Site::Github(GithubRoute::Repo(_))) => GITHUB_REPO_OVERLAY,
        Some(Site::Github(GithubRoute::PullRequest(_))) => GITHUB_PR_OVERLAY,
        Some(Site::Other) | None => OVERLAY,
    }
}
```

`crates/mercury/src/state/mod.rs`, in `overlay_content`, before:

```rust
            Self::Site(_) => site::overlay_for(
                foreground
                    .confirmed_chrome()
                    .and_then(|chrome| chrome.url.as_deref())
                    .map(Site::from_url),
            ),
```

after:

```rust
            Self::Site(_) => site::overlay_for(
                foreground
                    .confirmed_chrome()
                    .and_then(|chrome| chrome.url.as_deref())
                    .map(Site::from_url)
                    .as_ref(),
            ),
```

The three new files. `crates/mercury/src/state/overlays/github.txt`:

```
  GITHUB
  ────────────────────
  o    overlay
  t    typing
  esc  home
```

`crates/mercury/src/state/overlays/github-repo.txt`:

```
  GITHUB REPO
  ────────────────────
  c    clone to ~/code
  o    overlay
  t    typing
  esc  home
```

`crates/mercury/src/state/overlays/github-pull-request.txt`:

```
  GITHUB PR
  ────────────────────
  c    check out
  o    overlay
  t    typing
  esc  home
```

## Change 6: the bindings

Both run `gh`. It is on `PATH`, it is already how this machine talks to github, and it carries the credentials, so a private repository clones and a pull request from a fork checks out. The two commands are the ones you would type.

Both are one decision rather than something you repeat, so both go home.

`crates/mercury/src/handlers/github.rs`:

```rust
/// `c` on a repository page: clone it into `~/code/<name>`.
///
/// The destination is the repository's name without its owner, because that is where it would be
/// looked for. Cloning one that is already there fails, and `gh` says so in the log.
pub(crate) fn clone_repo<E>(
    _ev: &E,
    node: Node<GithubSitePath<'_>, GithubRepoSite>,
) -> Vec<MercuryEffect> {
    let Node { parent, data } = node;
    let root = parent.parent.ascend();
    let mut dest = root.code_dir.clone();
    dest.push(&data.repo.name);
    let run = MercuryEffect::Run(Run {
        program: "gh".to_owned(),
        args: vec![
            "repo".to_owned(),
            "clone".to_owned(),
            format!("{}/{}", data.repo.owner, data.repo.name),
            dest.to_string_lossy().into_owned(),
        ],
        cwd: root.code_dir.clone(),
    });
    and_go_home_from(root, [run])
}

/// `c` on a pull request page: check it out in the clone, by its number.
///
/// The number is all the model has and all `gh` needs: it resolves the branch itself, in the clone,
/// when it runs. A branch name read off the page would be a second copy of something github already
/// knows and this side would have to keep current.
///
/// The clone has to be there. When it is not, `gh` is run in a directory that does not exist and
/// the failure is in the log, which is the same place a clone's failure is.
pub(crate) fn checkout_pull_request<E>(
    _ev: &E,
    node: Node<GithubSitePath<'_>, GithubPullRequestSite>,
) -> Vec<MercuryEffect> {
    let Node { parent, data } = node;
    let root = parent.parent.ascend();
    let mut cwd = root.code_dir.clone();
    cwd.push(&data.repo.name);
    let run = MercuryEffect::Run(Run {
        program: "gh".to_owned(),
        args: vec![
            "pr".to_owned(),
            "checkout".to_owned(),
            data.number.to_string(),
        ],
        cwd,
    });
    and_go_home_from(root, [run])
}
```

Neither handler is generic over its path. A `Node` inside a `Node` is not something `Ascend` walks, so the ascent is spelled out: `parent` is `GithubSitePath`, `parent.parent` is the `SiteLayerPath`, and that ascends to the root. Naming the level in the signature is also what keeps a handler for one page from being bound on another.

`crates/mercury/src/handlers/mod.rs` gains `mod github;` and `pub(crate) use github::*;`.

## The tests

`crates/mercury/tests/transitions.rs` already has `site_showing(url)`. The table is every key in each of the four github states, and the assertion is the whole command:

```rust
const CODE: &str = "/Users/test/code";

fn gh(args: &[&str], cwd: &str) -> MercuryEffect {
    ran("gh", args, cwd)
}

#[test]
fn c_clones_the_repo_you_are_looking_at() {
    let mut m = site_showing("https://github.com/rbalicki2/freddie/tree/master/crates");
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        Some(vec![
            gh(
                &["repo", "clone", "rbalicki2/freddie", "/Users/test/code/freddie"],
                CODE,
            ),
            show_layer("Home"),
        ])
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn c_checks_out_the_pull_request_you_are_looking_at() {
    let mut m = site_showing("https://github.com/rbalicki2/freddie/pull/12/files");
    assert_eq!(
        m.handle(&key(Key::KeyC)),
        Some(vec![
            gh(&["pr", "checkout", "12"], "/Users/test/code/freddie"),
            show_layer("Home"),
        ])
    );
    assert!(matches!(m.layer(), Layer::Home(_)));
}

#[test]
fn c_is_unbound_where_there_is_no_repository() {
    for url in [
        "https://github.com",
        "https://github.com/rbalicki2",
        "https://github.com/settings/keys",
        "https://www.x.com/asdfasdf",
    ] {
        let mut m = site_showing(url);
        // Swallowed, and the site layer treats the keypress as activity: its timer resets.
        assert_eq!(m.handle(&key(Key::KeyC)), Some(vec![return_home_timer()]), "{url}");
        assert!(matches!(m.layer(), Layer::Site(_)), "{url}");
    }
}

#[test]
fn the_overlay_is_the_pages_keymap() {
    for (url, want) in [
        ("https://github.com", GITHUB_OVERLAY),
        ("https://github.com/rbalicki2/freddie", GITHUB_REPO_OVERLAY),
        ("https://github.com/rbalicki2/freddie/pull/12", GITHUB_PR_OVERLAY),
        ("https://claude.ai/new", CLAUDE_AI_OVERLAY),
        ("https://www.x.com/asdfasdf", SITE_OVERLAY),
    ] {
        let mut m = site_showing(url);
        assert_eq!(m.handle(&key(Key::KeyO)), Some(vec![show_overlay(want)]), "{url}");
    }
}
```

The keys the levels do not bind — `o`, `t`, `escape`, and everything unbound — keep asserting what they assert on every other site, from the site layer above.
