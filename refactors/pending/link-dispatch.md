# opening a link where it belongs

A GitHub URL for a work org needs the Chrome profile whose session has access to that org. macOS does not know that. LaunchServices hands every `http` and `https` open to the default browser, and Chrome opens it in whichever profile was last in front, so half of these links land in a profile that shows a 404 and the fix is opening the URL again by hand in the right window.

The decision is a function of the URL and what the machine already knows. Mercury holds what the machine knows, so mercury makes the decision.

## Being the thing links are delivered to

There is no way to observe a URL open from outside. LaunchServices delivers it, and it delivers only to a bundled app declaring the scheme in `CFBundleURLTypes`. So freddie becomes the default browser: a bundle with no window whose only job is to hand the URL to mercury.

That bundle is a source. It decides nothing, exactly as `freddie_app_nav`'s watcher decides nothing.

It is also the only piece that can be launched with mercury not running, so it carries one decision the model cannot make for it: with nothing listening on the socket, it opens the URL in Chrome itself. A link that vanishes is worse than a link in the wrong profile.

## What the model gains

Nothing. No field, no state. The route is a pure function of the URL and the front app, both of which are already there.

```
Freddie Links.app --frame--> mercury --OpenUrl--> open -n -b com.google.Chrome --args --profile-directory=...
```

## Change 1: opening a link is an effect

`freddie_app_nav` grows a second sink beside `foreground`, and the crate description becomes "App foregrounding, link opening, and frontmost-app watching for freddie".

`crates/freddie_app_nav/src/url.rs`:

```rust
/// A Chrome profile, named by its directory under `~/Library/Application Support/Google/Chrome`.
///
/// The directory and not the display name, because `--profile-directory` takes the directory:
/// the profile shown as "Work" lives in `Profile 1`. The mapping between the two is in Chrome's
/// `Local State`, and nothing here reads it; the directory is what `chrome://version` reports as
/// the Profile Path, and it is written down once, here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChromeProfile {
    Personal,
    Work,
}

impl ChromeProfile {
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Personal => "Default",
            Self::Work => "Profile 1",
        }
    }
}

/// An app that opens a link itself rather than a browser doing it.
///
/// Every variant has a bundle id, so a target cannot name an app there is no way to open. That is
/// why this is not `App`, whose `Other` has none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlApp {
    Zoom,
    Figma,
}

impl UrlApp {
    #[must_use]
    pub const fn bundle_id(self) -> &'static str {
        match self {
            Self::Zoom => "us.zoom.xos",
            Self::Figma => "com.figma.Desktop",
        }
    }
}

/// Where a link opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlTarget {
    ChromeProfile(ChromeProfile),
    App(UrlApp),
}
```

Both bundle ids are confirmed with `osascript -e 'id of app "Zoom"'` before this change lands, and corrected in place if either differs.

The sink, beside `foreground`:

```rust
/// Open `url` at `target`.
///
/// # Errors
///
/// If `open` cannot be spawned, or exits nonzero.
pub fn open_url(url: &str, target: UrlTarget) -> Result<(), NavError> {
    let status = Command::new("open")
        .args(open_url_args(url, target))
        .status()
        .map_err(NavError::Spawn)?;
    if status.success() {
        Ok(())
    } else {
        Err(NavError::Failed)
    }
}

/// The `open` arguments that put `url` at `target`.
///
/// A profile needs `--args`, because everything before it belongs to LaunchServices, which routes
/// by its own rules and would hand the URL straight back to whatever is registered for the scheme.
/// After `--args` the URL is Chrome's own command line, so Chrome opens it in the profile named
/// beside it. `-n` asks for a new instance; a running Chrome takes it over and opens the window in
/// that profile.
///
/// The profile flag is one argument, so a directory with a space in it needs no quoting: nothing
/// here goes through a shell.
fn open_url_args(url: &str, target: UrlTarget) -> Vec<String> {
    match target {
        UrlTarget::ChromeProfile(profile) => vec![
            "-n".to_owned(),
            "-b".to_owned(),
            "com.google.Chrome".to_owned(),
            "--args".to_owned(),
            format!("--profile-directory={}", profile.directory()),
            url.to_owned(),
        ],
        UrlTarget::App(app) => vec!["-b".to_owned(), app.bundle_id().to_owned(), url.to_owned()],
    }
}
```

The test:

```rust
#[test]
fn the_open_arguments_name_the_target() {
    for (target, want) in [
        (
            UrlTarget::ChromeProfile(ChromeProfile::Work),
            vec![
                "-n",
                "-b",
                "com.google.Chrome",
                "--args",
                "--profile-directory=Profile 1",
                "https://github.com/acme/thing",
            ],
        ),
        (
            UrlTarget::App(UrlApp::Zoom),
            vec!["-b", "us.zoom.xos", "https://github.com/acme/thing"],
        ),
    ] {
        assert_eq!(
            open_url_args("https://github.com/acme/thing", target),
            want,
            "{target:?}"
        );
    }
}
```

`MercuryEffect` gains the arm. Before:

```rust
    /// Put text on the clipboard, replacing what is there.
    Copy(Copied),
```

after:

```rust
    /// Put text on the clipboard, replacing what is there.
    Copy(Copied),
    /// Open a link, at the profile or app the handler picked.
    OpenUrl(OpenUrl),
```

with the payload carrying both halves, so performing it looks nothing up:

```rust
/// A link and where it opens.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct OpenUrl {
    pub url: String,
    pub target: UrlTarget,
}
```

Performed like the other subprocess effects, on its own thread. `perform_effect` gains:

```rust
        MercuryEffect::OpenUrl(OpenUrl { url, target }) => open_url(url, target),
```

```rust
/// Open a link, fire-and-forget on its own thread: it spawns `open`, which the effect loop should
/// no more wait on than it waits on a placement.
fn open_url(url: String, target: UrlTarget) {
    std::thread::spawn(move || match freddie_app_nav::open_url(&url, target) {
        Ok(()) => debug!(%url, ?target, "opened"),
        Err(e) => warn!(%url, ?target, error = %e, "open failed"),
    });
}
```

## Change 2: mercury routes a link it is told about

The vocabulary an outside process may speak grows one variant. `external.rs`, before:

```rust
pub enum IncomingEvent {
    /// The front browser tab's URL changed.
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
}
```

after:

```rust
pub enum IncomingEvent {
    /// The front browser tab's URL changed.
    #[serde(rename = "IncomingEvent.Tab")]
    Tab(TabMessage),
    /// Something asked the machine to open a link, and freddie is the default browser.
    #[serde(rename = "IncomingEvent.Link")]
    Link(LinkMessage),
}

#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct LinkMessage {
    pub url: String,
}
```

and `on_message` gains its arm:

```rust
        Ok(IncomingEvent::Link(LinkMessage { url })) => {
            debug!(%url, "link");
            let _ = event_tx.send(link(url));
        }
```

The trigger and event, in `sources.rs` beside `Tabbed`:

```rust
/// A trigger that matches any link handed to the machine, whichever URL it carries.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Linked;

/// A link the machine was asked to open, from the link handler over the event socket.
///
/// Distinct from [`TabEvent`], which reports where a browser already is. This one is a request
/// that has not happened yet, and nothing opens until a handler says where.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct LinkEvent {
    pub url: String,
}
impl EventTrigger for Linked {
    type Event = LinkEvent;
    fn is_matching(&self, _ev: &LinkEvent) -> bool {
        true
    }
}
```

`MercuryEvent` gains `Link(LinkEvent)`, and the root binds it. Before:

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
```

after:

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    Linked => open_link,
```

The route is where the whole feature lives. `crates/mercury/src/state/link.rs`:

```rust
/// Where a link opens, given the URL alone.
///
/// Total: every URL has a destination, because a link with nowhere to go is a link that disappears
/// when someone clicks it. Anything unmatched opens in [`ChromeProfile::Personal`].
#[must_use]
pub fn route(url: &str) -> UrlTarget {
    match (site_host(url), github_owner(url)) {
        (Some("github.com"), Some("acme" | "acme-labs")) => {
            UrlTarget::ChromeProfile(ChromeProfile::Work)
        }
        (Some("zoom.us"), _) => UrlTarget::App(UrlApp::Zoom),
        (Some("figma.com"), _) => UrlTarget::App(UrlApp::Figma),
        _ => UrlTarget::ChromeProfile(ChromeProfile::Personal),
    }
}
```

`site_host` is the existing one in `sources.rs`, made `pub(crate)`. `github_owner` is its path-side counterpart, next to `host`:

```rust
/// The first path segment of `url`, which on GitHub is the owner: `https://github.com/acme/thing`
/// is `acme`.
///
/// `None` when there is no segment at all, which is `https://github.com` and `https://github.com/`.
/// The segment is returned as it appears; GitHub owners are case-insensitive, so a caller matching
/// on one lowercases first.
#[must_use]
pub fn first_path_segment(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let path = after_scheme.split_once('/')?.1;
    let segment = path
        .find(['/', '?', '#'])
        .map_or(path, |end| &path[..end]);
    (!segment.is_empty()).then_some(segment)
}
```

with `github_owner` in `state/link.rs` doing the lowercasing:

```rust
/// The owner a GitHub URL names, lowercased, so `/Acme/thing` and `/acme/thing` route alike.
fn github_owner(url: &str) -> Option<String> {
    first_path_segment(url).map(str::to_lowercase)
}
```

The handler, `crates/mercury/src/handlers/link.rs`:

```rust
/// Something asked the machine to open a link: open it where the route says.
///
/// Reads no state, so it is the route and nothing else. The front app joins it in
/// `link-dispatch.md`'s later change.
pub(crate) fn open_link(ev: &LinkEvent, _node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    vec![MercuryEffect::OpenUrl(OpenUrl {
        url: ev.url.clone(),
        target: route(&ev.url),
    })]
}
```

The tables, in `state/link.rs`:

```rust
#[test]
fn a_work_org_goes_to_the_work_profile() {
    for (url, want) in [
        (
            "https://github.com/acme/thing",
            UrlTarget::ChromeProfile(ChromeProfile::Work),
        ),
        (
            "https://github.com/acme/thing/pull/12#issuecomment-9",
            UrlTarget::ChromeProfile(ChromeProfile::Work),
        ),
        // The owner is case-insensitive on GitHub, so the route is too.
        (
            "https://github.com/Acme/thing",
            UrlTarget::ChromeProfile(ChromeProfile::Work),
        ),
        (
            "https://www.github.com/acme-labs/thing",
            UrlTarget::ChromeProfile(ChromeProfile::Work),
        ),
        // Somebody else's org, and freddie's own repo, are not work.
        (
            "https://github.com/freddiehg/freddie",
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        // A prefix of a work org is not that org.
        (
            "https://github.com/acmecorp/thing",
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        // The owner appearing anywhere but the first segment is not the owner.
        (
            "https://github.com/freddiehg/acme",
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        // A host ending the right way is not the host.
        (
            "https://github.com.evil.com/acme/thing",
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        ("https://zoom.us/j/123", UrlTarget::App(UrlApp::Zoom)),
        (
            "https://www.figma.com/file/abc",
            UrlTarget::App(UrlApp::Figma),
        ),
        (
            "https://news.ycombinator.com",
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        // No host at all still opens somewhere.
        ("about:blank", UrlTarget::ChromeProfile(ChromeProfile::Personal)),
    ] {
        assert_eq!(route(url), want, "{url}");
    }
}
```

and the segment split, beside the `host` tests in `sources.rs`:

```rust
#[test]
fn the_first_path_segment_is_the_owner() {
    for (url, want) in [
        ("https://github.com/acme/thing", Some("acme")),
        ("https://github.com/acme", Some("acme")),
        ("https://github.com/acme/", Some("acme")),
        ("https://github.com/acme?tab=repositories", Some("acme")),
        ("https://github.com/acme#readme", Some("acme")),
        ("https://github.com/", None),
        ("https://github.com", None),
        ("about:blank", None),
    ] {
        assert_eq!(first_path_segment(url), want, "{url}");
    }
}
```

The wire test joins the ones in `external.rs`:

```rust
#[test]
fn a_link_frame_carries_its_url() {
    let frame = r#"{"kind":"IncomingEvent.Link","value":{"url":"https://github.com/acme/thing"}}"#;
    let IncomingEvent::Link(link) =
        serde_json::from_str::<IncomingEvent>(frame).expect("a link frame deserializes")
    else {
        panic!("a link frame is a link");
    };
    assert_eq!(link.url, "https://github.com/acme/thing");
}
```

### Sending one from the terminal

Both the CLI verb and the bundle send the same frame, so the send is one function. `freddie_event_socket` gains it:

```rust
/// Connect to `127.0.0.1:port`, send `frame`, and close.
///
/// One connection per frame: a sender that lives longer than the daemon would need reconnection
/// logic, and mercury is restarted every time it is rebuilt.
///
/// # Errors
///
/// If nothing is listening, the handshake fails, or the frame cannot be written.
pub async fn send(port: u16, frame: &str) -> Result<(), tungstenite::Error> {
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}")).await?;
    ws.send(Message::text(frame.to_owned())).await?;
    ws.close(None).await
}
```

A native client sends no `Origin`, which `origin_allowed` already admits.

`Verb` gains its variant, in help order after `Uninstall`:

```rust
    /// Open a link through the running daemon, which decides where it goes.
    OpenUrl(OpenUrlArgs),
```

```rust
/// What `mercury open-url` can be told.
#[derive(clap::Args, Debug, PartialEq, Eq)]
pub struct OpenUrlArgs {
    /// The link to open.
    pub url: String,

    /// The loopback port the running daemon listens on.
    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
    pub port: u16,
}
```

The verb serializes an `IncomingEvent.Link` frame, sends it, and reports. A refused connection is `error!` and a nonzero exit, because the link did not open and the person who typed it needs to know.

After this change, with a daemon running:

```
mercury open-url https://github.com/acme/thing
```

opens a work-profile window, and the log holds the dispatch record.

## Change 3: the bundle that receives them

A new crate, `crates/freddie_link_handler`, producing one binary. It is the first subclass of an ObjC class in the workspace, so it keeps its own lint table with `unsafe_code = "deny"` and a SAFETY comment per call, the way `freddie_app_nav` does.

```rust
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FreddieLinkDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        /// LaunchServices delivered one or more URLs. It delivers to a running instance too, so
        /// this fires again for every later link without the app being launched twice.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            for url in urls {
                if let Some(url) = url.absoluteString() {
                    hand_over(&url.to_string());
                }
            }
        }
    }
);
```

`main` sets the activation policy to `Prohibited`, installs the delegate, and runs. `LSUIElement` in the plist keeps it out of the Dock and stops it stealing focus, so the app that was frontmost when the link was clicked still is when mercury dispatches.

```rust
/// Send the URL to mercury, or open it in Chrome if mercury does not answer.
///
/// The one decision this process makes. A link that goes nowhere because the daemon is stopped is
/// the worst failure this feature has, since the bundle owns every link on the machine.
///
/// The fallback names Chrome by bundle id, so LaunchServices hands the URL to Chrome rather than
/// to whatever is registered for the scheme, which is this process.
fn hand_over(url: &str) {
    match runtime.block_on(freddie_event_socket::send(DEFAULT_PORT, &frame(url))) {
        Ok(()) => debug!(%url, "handed over"),
        Err(e) => {
            warn!(%url, error = %e, "mercury did not answer: opening in Chrome");
            fall_back(url);
        }
    }
}
```

Everything it says goes through `tracing`, into the same log file, so a link that went to the wrong place is reconstructable from the one file. Its records carry its own pid, which is what tells them apart from the daemon's.

The bundle it goes in:

```
Freddie Links.app/
  Contents/
    Info.plist
    MacOS/
      freddie-link-handler
```

```xml
<key>CFBundleIdentifier</key>      <string>hg.freddie.links</string>
<key>CFBundleName</key>            <string>Freddie Links</string>
<key>CFBundleExecutable</key>      <string>freddie-link-handler</string>
<key>CFBundlePackageType</key>     <string>APPL</string>
<key>LSUIElement</key>             <true/>
<key>CFBundleURLTypes</key>
<array>
  <dict>
    <key>CFBundleURLName</key>     <string>Web site URL</string>
    <key>CFBundleTypeRole</key>    <string>Viewer</string>
    <key>CFBundleURLSchemes</key>
    <array>
      <string>http</string>
      <string>https</string>
    </array>
  </dict>
</array>
```

## Change 4: installing it, and taking it back out

`mercury browser install` assembles the bundle at `~/Applications/Freddie Links.app` from the binary beside the running `mercury`, writes `Info.plist` with the `plist` crate that `install_agent` already uses, and registers it:

```
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f ~/Applications/Freddie\ Links.app
```

Selecting it as the default browser is the user's, in System Settings under Desktop & Dock. `NSWorkspace`'s `setDefaultApplication(at:toOpenURLsWithScheme:)` exists and returns permission errors on current macOS, so the verb does not call it. It opens the pane instead:

```
open "x-apple.systempreferences:com.apple.Desktop-Settings.extension"
```

and says, at `info!`, which app to pick.

`mercury browser status` reports whether the bundle is installed and whether it is the current handler, read from the LaunchServices preference:

```
defaults read ~/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure
```

matching the `LSHandlerURLScheme` entry for `https` against `hg.freddie.links`.

`mercury browser uninstall` removes the bundle and runs `lsregister -u`. If the read above says freddie is still the handler, it says so at `warn!` and names the pane, because deleting the registered handler leaves the machine dispatching links to something that is gone.

## Change 5: the route reads the front app

`route` gains the argument, and the only reason it can is that `Foreground::app` is already there.

Before:

```rust
pub fn route(url: &str) -> UrlTarget
```

after:

```rust
/// Where a link opens, given the URL and the app it was clicked in.
///
/// The front app is the clicking app: the handler bundle is `LSUIElement`, so it does not take
/// focus, and the model's idea of what is frontmost still names whoever was clicked in.
pub fn route(url: &str, front: App) -> UrlTarget
```

with the arm that needs it:

```rust
        // Anything unmatched clicked in a work app is work.
        (_, _) if front == App::Slack => UrlTarget::ChromeProfile(ChromeProfile::Work),
```

`App` gains `Slack`, and both halves of the bundle-id round trip:

```rust
            "com.tinyspeck.slackmacgap" => Self::Slack,
```

```rust
            Self::Slack => Some("com.tinyspeck.slackmacgap"),
```

The handler reads it off the model:

```rust
pub(crate) fn open_link(ev: &LinkEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let front = node.parent.foreground.app();
    vec![MercuryEffect::OpenUrl(OpenUrl {
        url: ev.url.clone(),
        target: route(&ev.url, front),
    })]
}
```

and the table gains the pairs it now distinguishes:

```rust
#[test]
fn the_clicking_app_decides_what_the_url_does_not() {
    for (url, front, want) in [
        (
            "https://news.ycombinator.com",
            App::Slack,
            UrlTarget::ChromeProfile(ChromeProfile::Work),
        ),
        (
            "https://news.ycombinator.com",
            App::Ghostty,
            UrlTarget::ChromeProfile(ChromeProfile::Personal),
        ),
        // The URL wins where it says something: a Zoom link opens Zoom from anywhere.
        ("https://zoom.us/j/123", App::Slack, UrlTarget::App(UrlApp::Zoom)),
    ] {
        assert_eq!(route(url, front), want, "{url} from {front:?}");
    }
}
```
