# nothing else starts playing while you are in a meeting

A phone call arrives during a Google Meet call. macOS pauses whatever holds Now Playing, and when the phone call ends it issues a play. Now Playing is held by whichever tab made sound most recently, which is a YouTube tab from an hour ago rather than the meeting. So the call ends and YouTube is playing into a meeting that is still running.

Neither Chrome nor macOS marks that resume as something the system did. What distinguishes it is the situation: a tab starts making sound, a meeting is live, and the tab is not the meeting.

So that is the rule. While a meeting tab exists, any other tab that starts making sound is muted, once, and the log says which.

It only ever mutes. Unmuting is the speaker icon in Chrome's tab strip, and a tab unmuted by hand is not muted again for the rest of that meeting.

## What this lands after

`external-effects.md`, for `TabId`, `BrowserCommand`, `Command`, and `MercuryEffect::Browser`. `extension-commands.md`, for the extension reading a command frame and acting on it. Every type from those two is used here as it stands once they have shipped.

## What the model holds

```rust
/// Every tab Chrome has, as of the last snapshot, and who has been silenced this meeting.
#[derive(Debug, Default)]
pub struct Tabs {
    /// The last snapshot, by id. Replaced whole on every report.
    open: BTreeMap<TabId, Tab>,
    /// Tabs muted during the meeting that is running now, emptied when it ends.
    ///
    /// This is what stops a fight with the user: a tab muted here, unmuted by hand, and then
    /// audible again is left alone, because muting it a second time would be mercury insisting.
    silenced: BTreeSet<TabId>,
}

/// One tab, as the extension reports it.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Clone, Debug)]
pub struct Tab {
    pub url: String,
    /// Whether the tab is making sound right now, which is Chrome's own `audible`.
    pub audible: bool,
}
```

`Mercury` gains the field:

```rust
    /// Every tab Chrome has. Fed by the extension, read by nothing but the mute rule.
    pub tabs: Tabs,
```

## Change 1: the extension reports every tab, not just the front one

`IncomingEvent` gains a variant beside `Tab`, which stays as it is: it answers "what am I looking at", and this one answers "what is Chrome doing".

```rust
    /// Every tab Chrome has, whenever any of them changes.
    #[serde(rename = "IncomingEvent.Tabs")]
    Tabs(TabsMessage),
```

```rust
#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct TabsMessage {
    pub tabs: Vec<TabReport>,
}

/// One tab in a snapshot.
#[derive(serde::Deserialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct TabReport {
    pub id: TabId,
    pub url: String,
    pub audible: bool,
}
```

A snapshot and not a delta, for the reason `seed-at-construction.md` gives: applying it twice lands where applying it once does, because the second application finds every tab already in the state it reports and produces no transition. A stream of "this tab became audible" messages has no such property, and a missed one leaves the model believing something that is not true until Chrome restarts.

`background.ts` gains the snapshot and the listeners that push it:

```ts
/** Every tab Chrome has, as mercury's wire type. Tabs with no id or no URL are not reportable. */
async function snapshot(): Promise<TabReport[]> {
  const tabs = await chrome.tabs.query({});
  return tabs.flatMap((tab) =>
    tab.id === undefined || tab.url === undefined || tab.url === ""
      ? []
      : [{ id: tab.id, url: tab.url, audible: tab.audible ?? false }],
  );
}

/** Send the whole snapshot to mercury. */
async function pushTabs(): Promise<void> {
  const frame: IncomingEvent = {
    kind: "IncomingEvent.Tabs",
    value: { tabs: await snapshot() },
  };
  send(JSON.stringify(frame));
}
```

`pushUrl`'s send-or-queue-on-open body becomes `send`, called by both, so there is one place that knows how a frame reaches the socket.

The listeners: audibility and URL are the two fields the rule reads, and a tab going away has to leave the snapshot.

```ts
// Any tab, not just the active one: the tab that starts playing is by definition not the one
// being looked at. `audible` and `url` are the only fields the snapshot carries.
chrome.tabs.onUpdated.addListener((_tabId, info) => {
  if (info.audible !== undefined || info.url !== undefined) void pushTabs();
});

chrome.tabs.onRemoved.addListener(() => {
  void pushTabs();
});
```

## Change 2: the rule

`crates/mercury/src/state/tabs.rs`. Pure, so the whole file is a test table.

A meeting is a `meet.google.com` URL whose path is a meeting code, which is Google's three-four-three: `https://meet.google.com/abc-defg-hij`. The bare host is the landing page and is not a meeting.

```rust
/// Whether `url` is a Google Meet call, as opposed to Meet's landing page.
///
/// The code is Meet's own shape: three letters, four, three, lowercase, joined by hyphens. A
/// hand-rolled check rather than a regex dependency for one pattern.
#[must_use]
pub fn is_meeting(url: &str) -> bool {
    if site_host(url) != Some("meet.google.com") {
        return false;
    }
    let Some(code) = first_path_segment(url) else {
        return false;
    };
    let mut parts = code.split('-');
    let shape = [3, 4, 3].into_iter().all(|len| {
        parts
            .next()
            .is_some_and(|part| part.len() == len && part.chars().all(|c| c.is_ascii_lowercase()))
    });
    shape && parts.next().is_none()
}
```

`first_path_segment` is the one `link-dispatch.md` adds; whichever of the two lands first writes it.

The snapshot goes in, and the tabs to mute come out:

```rust
impl Tabs {
    /// Replace the snapshot, returning every tab that just started making sound and should not
    /// have.
    ///
    /// A tab qualifies when it was silent in the previous snapshot, is audible in this one, a
    /// meeting is live, it is not that meeting, and it has not already been silenced during this
    /// meeting.
    #[must_use]
    pub fn report(&mut self, reported: Vec<TabReport>) -> Vec<TabId> {
        let open: BTreeMap<TabId, Tab> = reported
            .into_iter()
            .map(|r| (r.id, Tab { url: r.url, audible: r.audible }))
            .collect();

        let meeting = open
            .iter()
            .find_map(|(id, tab)| is_meeting(&tab.url).then_some(*id));

        // No meeting: nothing is protected, and the next one starts with a clean slate.
        let Some(meeting) = meeting else {
            self.open = open;
            self.silenced.clear();
            return Vec::new();
        };

        let started: Vec<TabId> = open
            .iter()
            .filter(|(id, tab)| {
                tab.audible
                    && **id != meeting
                    && !self.silenced.contains(id)
                    && !self.open.get(id).is_some_and(|was| was.audible)
            })
            .map(|(id, _)| *id)
            .collect();

        self.silenced.extend(started.iter().copied());
        self.open = open;
        started
    }
}
```

A tab mercury has never seen before that arrives already audible counts as having started, which is the case where the tab was opened in the background and began playing on its own.

The handler, `crates/mercury/src/handlers/tabs.rs`:

```rust
/// Chrome reported its tabs: record them, and mute anything that started playing over a meeting.
pub(crate) fn record_tabs(ev: &TabsEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.tabs
        .report(ev.tabs.clone())
        .into_iter()
        .map(|tab| {
            info!(?tab, "muting a tab that started playing during a meeting");
            MercuryEffect::Browser(BrowserCommand { tab, command: Command::MuteTab })
        })
        .collect()
}
```

The root binds it beside the others:

```rust
#[bind(
    Foregrounded => record_front_app,
    Tabbed => record_tab_url,
    TabsReported => record_tabs,
```

with `TabsReported` and `TabsEvent` in `sources.rs`, in the shape `Tabbed` and `TabEvent` already have.

## Change 3: the command

`Command`, before:

```rust
pub enum Command {
    /// Put claude.ai's settings up in the named tab.
    #[serde(rename = "Command.OpenClaudeSettings")]
    OpenClaudeSettings,
}
```

after:

```rust
pub enum Command {
    /// Put claude.ai's settings up in the named tab.
    #[serde(rename = "Command.OpenClaudeSettings")]
    OpenClaudeSettings,
    /// Mute the named tab. Nothing unmutes it: that is the speaker icon in the tab strip.
    #[serde(rename = "Command.MuteTab")]
    MuteTab,
}
```

and the extension performs it with the `tabs` permission the manifest already asks for:

```ts
case "Command.MuteTab":
  void chrome.tabs.update(command.tab, { muted: true });
  return;
```

## The tests

```rust
fn report(id: i32, url: &str, audible: bool) -> TabReport {
    TabReport { id: TabId(id), url: url.to_owned(), audible }
}

const MEETING: &str = "https://meet.google.com/abc-defg-hij";
const YOUTUBE: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

#[test]
fn a_tab_that_starts_playing_over_a_meeting_is_muted_once() {
    let mut tabs = Tabs::default();
    // The meeting, and a silent YouTube tab.
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, false)]),
        Vec::new()
    );
    // The phone call ends and YouTube resumes.
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, true)]),
        vec![TabId(2)]
    );
    // Muted, so the next snapshot has it silent, and unmuting it by hand does not re-mute it.
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, false)]),
        Vec::new()
    );
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, true)]),
        Vec::new()
    );
}

#[test]
fn the_meeting_itself_is_never_muted() {
    let mut tabs = Tabs::default();
    assert_eq!(tabs.report(vec![report(1, MEETING, false)]), Vec::new());
    assert_eq!(tabs.report(vec![report(1, MEETING, true)]), Vec::new());
}

#[test]
fn nothing_is_muted_with_no_meeting_open() {
    let mut tabs = Tabs::default();
    assert_eq!(tabs.report(vec![report(2, YOUTUBE, false)]), Vec::new());
    assert_eq!(tabs.report(vec![report(2, YOUTUBE, true)]), Vec::new());
}

#[test]
fn the_meeting_ending_clears_who_was_silenced() {
    let mut tabs = Tabs::default();
    let _ = tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, false)]);
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, true)]),
        vec![TabId(2)]
    );
    // The meeting tab closes, then a new meeting starts and YouTube pipes up again.
    let _ = tabs.report(vec![report(2, YOUTUBE, false)]);
    let _ = tabs.report(vec![report(3, MEETING, true), report(2, YOUTUBE, false)]);
    assert_eq!(
        tabs.report(vec![report(3, MEETING, true), report(2, YOUTUBE, true)]),
        vec![TabId(2)]
    );
}

#[test]
fn a_tab_arriving_already_playing_counts_as_starting() {
    let mut tabs = Tabs::default();
    let _ = tabs.report(vec![report(1, MEETING, true)]);
    assert_eq!(
        tabs.report(vec![report(1, MEETING, true), report(2, YOUTUBE, true)]),
        vec![TabId(2)]
    );
}

#[test]
fn the_same_snapshot_twice_mutes_nothing_the_second_time() {
    let mut tabs = Tabs::default();
    let snapshot = || vec![report(1, MEETING, true), report(2, YOUTUBE, true)];
    assert_eq!(tabs.report(snapshot()), vec![TabId(2)]);
    assert_eq!(tabs.report(snapshot()), Vec::new());
}

#[test]
fn only_a_meeting_code_is_a_meeting() {
    for (url, want) in [
        ("https://meet.google.com/abc-defg-hij", true),
        ("https://meet.google.com/abc-defg-hij?authuser=0", true),
        // The landing page, and the parts of Meet that are not a call.
        ("https://meet.google.com", false),
        ("https://meet.google.com/", false),
        ("https://meet.google.com/landing", false),
        // Nearly the code, and not.
        ("https://meet.google.com/ab-defg-hij", false),
        ("https://meet.google.com/abc-defg-hij-klm", false),
        ("https://meet.google.com/ABC-DEFG-HIJ", false),
        // A host ending the right way is not the host.
        ("https://meet.google.com.evil.com/abc-defg-hij", false),
        ("https://www.youtube.com/watch?v=x", false),
    ] {
        assert_eq!(is_meeting(url), want, "{url}");
    }
}
```
