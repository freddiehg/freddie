# commands scoped to the state that emits them

`external-effects.md` ships one flat `Command` enum for the whole process. Any handler on any level can build any variant of it, so "settings is a claude.ai command" is a convention rather than something the types say. When x.com's level arrives with a list of its own, a handler on claude.ai will be able to emit "like the selected post", and nothing will stop it.

This makes the vocabulary follow the state tree. A level owns an enum of the commands it may emit, the enums nest the way the levels nest, and the only route from a level's enum to a `MercuryEffect` runs through a value of that level. A handler that is not on the level cannot reach the route, so it cannot name the command.

This is thiserror's shape rather than anyhow's: every level keeps its own enum, and a `From` per hop composes them into the one type the wire speaks. ts-rs exports the nest, so the extension's claude.ai code takes a `ClaudeAiCommand` and cannot be handed x.com's.

## The nest

`Command` becomes `SiteCommand`, one variant per site level that has commands, before:

```rust
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind", content = "value")]
pub enum Command {
    #[serde(rename = "Command.OpenClaudeSettings")]
    OpenClaudeSettings,
}
```

after:

```rust
/// What the site in a tab can be asked to do. One variant per site level that has commands, so
/// this enum splits exactly when the site levels split.
///
/// `derive_more::From` gives `From<ClaudeAiCommand>`, which is the hop each level's `Commands`
/// impl relies on to reach the wire type without naming the nest itself.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(serde::Serialize, Debug, derive_more::From)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum SiteCommand {
    #[serde(rename = "SiteCommand.ClaudeAi")]
    ClaudeAi(ClaudeAiCommand),
}

/// claude.ai's commands. Emitted by `ClaudeAiSite` and by nothing else.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum ClaudeAiCommand {
    #[serde(rename = "ClaudeAiCommand.OpenSettings")]
    OpenSettings(SettingsSection),
}

/// Which settings page. The extension maps this to the site's route, so mercury names a section
/// and never spells a URL.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Clone, Copy, serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
#[serde(tag = "kind", content = "value")]
pub enum SettingsSection {
    #[serde(rename = "SettingsSection.General")]
    General,
}
```

The frame the extension reads:

```jsonc
// mercury -> client
{
  "kind": "OutgoingEffect.Command",
  "value": {
    "tab": 42,
    "command": {
      "kind": "SiteCommand.ClaudeAi",
      "value": {
        "kind": "ClaudeAiCommand.OpenSettings",
        "value": { "kind": "SettingsSection.General", "value": null }
      }
    }
  }
}
```

## The seal

`BrowserCommand`'s fields close, so the struct can only be built where the trait's default body builds it. Before:

```rust
pub struct BrowserCommand {
    pub tab: TabId,
    pub command: Command,
}
```

after:

```rust
/// A command and the tab it is addressed to.
///
/// The fields are private, and the only constructor is [`Commands::emit`], whose body is here in
/// this module. So the way to produce one is to hold the level that owns the command: no other
/// module in the crate can assemble a pair, and outside the crate there is not even a field to
/// name.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(serde::Serialize, Debug)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "../../../chrome-extension/src/wire/")
)]
pub struct BrowserCommand {
    tab: TabId,
    command: SiteCommand,
}

impl BrowserCommand {
    /// The tab this command is addressed to.
    #[must_use]
    pub const fn tab(&self) -> TabId {
        self.tab
    }

    /// What that tab is being asked to do.
    #[must_use]
    pub const fn command(&self) -> &SiteCommand {
        &self.command
    }
}

/// A level's command vocabulary: what that level, and only that level, may ask the browser to do.
///
/// Implemented by a state struct that resolved from a tab, which is what makes `tab` infallible:
/// the level exists because a tab was reported, so there is no "which tab" question left for a
/// handler to get wrong or to have to give up on.
pub trait Commands {
    /// The commands this level emits.
    type Command: Into<SiteCommand>;

    /// The tab this level resolved from. Every command it emits is addressed there.
    fn tab(&self) -> TabId;

    /// Address `command` to this level's tab, as an effect for the effect loop to write.
    ///
    /// An impl may override this, and gains nothing by it: `BrowserCommand`'s fields are private
    /// to this module, so an override has no way to build one.
    fn emit(&self, command: Self::Command) -> MercuryEffect {
        MercuryEffect::Browser(BrowserCommand {
            tab: self.tab(),
            command: command.into(),
        })
    }
}
```

`Browser::send` reads the fields directly, since it is in this module:

```rust
-        let tab = command.tab;
+        let tab = command.tab();
```

## The level carries its tab

`ClaudeAiSite` holds the tab it resolved from, before:

```rust
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => new_chat,
    Key::KeyS.down() => open_settings,
)]
pub struct ClaudeAiSite {}
```

after:

```rust
/// claude.ai's level, where `n` starts a new chat and `s` puts settings up.
///
/// It carries the tab it resolved from. The field is private, so this module is the only place a
/// `ClaudeAiSite` is built, and [`site_data`] builds it from the tab whose URL matched. A handler
/// is handed one; nothing else can make one.
#[derive(Bind, Debug)]
#[derived_node(parent = SiteLayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::KeyN.down() => new_chat,
    Key::KeyS.down() => open_settings,
)]
pub struct ClaudeAiSite {
    tab: TabId,
}

impl Commands for ClaudeAiSite {
    type Command = ClaudeAiCommand;

    fn tab(&self) -> TabId {
        self.tab
    }
}
```

`site_data` builds it from the report it matched on, before:

```rust
fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    let root = path.parent().parent();
    let url = &root.foreground.confirmed_chrome()?.tab.as_ref()?.url;
    match Site::from_url(url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite {})),
        Site::Other => None,
    }
}
```

after:

```rust
fn site_data(path: &SiteLayerPath) -> Option<SiteData> {
    // SiteLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    let front = root.foreground.confirmed_chrome()?.tab.as_ref()?;
    match Site::from_url(&front.url) {
        Site::ClaudeAi => Some(SiteData::ClaudeAi(ClaudeAiSite { tab: front.id })),
        Site::Other => None,
    }
}
```

The URL and the id come from one `FrontTab`, so the level cannot be built against one tab's URL and another's id.

## The handler

Before, generic over the node, ascending to the root for a tab that might not be there:

```rust
pub(crate) fn open_settings<'a, E, P: Ascend<MercuryPath<'a>>, D>(
    _ev: &E,
    node: Node<P, D>,
) -> Vec<MercuryEffect> {
    let root: &mut Mercury = node.parent.ascend();
    let Some(front) = root
        .foreground
        .confirmed_chrome()
        .and_then(|chrome| chrome.tab.as_ref())
    else {
        return Vec::new();
    };
    vec![MercuryEffect::Browser(BrowserCommand {
        tab: front.id,
        command: Command::OpenClaudeSettings,
    })]
}
```

after, naming the level it is bound on:

```rust
/// `s` on claude.ai: put settings up.
///
/// The node is `ClaudeAiSite`, so this handler can be bound on that level and nowhere else, and
/// the commands it can name are that level's. There is no "no tab" case left: the level resolved
/// from one.
pub(crate) fn open_settings<E>(
    _ev: &E,
    node: Node<SiteLayerPath<'_>, ClaudeAiSite>,
) -> Vec<MercuryEffect> {
    vec![
        node.data
            .emit(ClaudeAiCommand::OpenSettings(SettingsSection::General)),
    ]
}
```

`new_chat` stays generic over the node: it emits a keystroke and reads nothing.

## What holds this together

- `BrowserCommand`'s fields are private to `external`, so `MercuryEffect::Browser` cannot be built anywhere else in the crate, whatever a handler imports.
- `Commands::emit` takes `&self`, so producing one needs the level's value. A handler gets that value from the derive, as `node.data`, and only on the level it is bound on.
- `ClaudeAiSite::tab` is private to `state::site`, so `site_data` is the only place a `ClaudeAiSite` comes from, and a handler elsewhere cannot conjure one to call `emit` on.
- `Site::from_url` decides which level resolves, so which commands are reachable follows from the tab's URL and nothing else.

The tree it produces, once x.com's level lands: `XSite` implements `Commands` with `type Command = XCommand`, `SiteCommand` gains `X(XCommand)` and the `From` with it, and no existing level changes. A level that splits splits its enum with it.

`bind` and `laserbeam` are untouched. Handlers still return `Vec<MercuryEffect>`, and dispatch still carries one output type; the scoping is in what a handler can construct, not in what it returns.

## Changes

1. `SiteCommand`, `ClaudeAiCommand`, `SettingsSection`, and `Command`'s removal.
2. `BrowserCommand`'s private fields, its two accessors, and the `Commands` trait.
3. `ClaudeAiSite`'s `tab` field, its `Commands` impl, `site_data` building it, and `open_settings` taking the level.

## Tests

In `crates/mercury/tests/transitions.rs`, through the accessors, since a test crate cannot build a `BrowserCommand`:

- `u` then `s` on a reported claude.ai tab produces one `MercuryEffect::Browser` whose `tab()` is the reported id and whose `command()` is `SiteCommand::ClaudeAi(ClaudeAiCommand::OpenSettings(SettingsSection::General))`.
- A tab report for a second tab, then `s`, addresses the second id: the level is rebuilt from the report on every dispatch, so nothing has to be resynced.
- `u` then `s` while Chrome is front with no tab reported is unbound and produces nothing, and no site level resolved.

In `crates/mercury/src/external.rs`:

- The nested frame above is what `serde_json::to_string` writes for that command, byte for byte.
- `ClaudeAiCommand::OpenSettings(SettingsSection::General).into()` is `SiteCommand::ClaudeAi(..)`, so the hop the trait bound relies on is the one `derive_more` derived.
