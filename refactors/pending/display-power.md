# turning a display off

## What "off" means

A display that is off is not a display that is asleep and not one at zero brightness. It is gone from the arrangement. macOS drops it from `NSScreen.screens` and from `CGGetActiveDisplayList`, posts `NSApplicationDidChangeScreenParametersNotification`, moves the windows that were on it onto a display that remains, and moves the menu bar and the dock if it was the one carrying them. A MacBook whose built-in display is off is in the state closed-lid clamshell puts it in, with the lid open.

Coming back is the same in reverse: the display reappears in the arrangement, and the windows macOS moved off it stay where they were moved to. Nothing restores a window to the display it came from; `window-restore.md` is where that would live.

## What macOS offers

Nothing public. `CGDisplayCapture` takes a display over for one process rather than removing it, mirroring keeps it in the arrangement, and `DisplayServices` brightness leaves it there at zero. There is no documented call that removes a connected display from the arrangement and puts it back.

BetterDisplay does it with private calls. Its documentation states the constraints and not the symbols:

- Apple Silicon needs macOS Ventura or later. On Intel it is called experimental.
- The display's connection has to be "connection management capable".
- It is a Pro feature.

The specific private interface is undocumented and not verified here. Nothing in this plan reimplements it; freddie asks BetterDisplay to do it.

Sources: [BetterDisplay](https://github.com/waydabber/BetterDisplay), [Integration features, CLI](https://github.com/waydabber/BetterDisplay/wiki/Integration-features,-CLI), [issue 1663, disconnect built-in when an external is connected](https://github.com/waydabber/BetterDisplay/issues/1663), [issue 4391, condition it on a specific external](https://github.com/waydabber/BetterDisplay/issues/4391).

## Whether freddie should do this at all

BetterDisplay already has the rule "disconnect the built-in display when an external display is connected" as a setting. If that is the rule you want, turn it on and write none of this. It is one checkbox against several hundred lines here, and it survives a mercury that is not running.

The reason to put it in freddie is a rule BetterDisplay's setting cannot state: off for the desk monitor and not for the projector in a conference room, off only above some size, off only while a particular app is frontmost. Those are conditions on the model, and the model is here. Everything below is that version.

## What it already does to the model

Nothing has to be built to observe this. `freddie_windows` observes `NSApplicationDidChangeScreenParametersNotification`, re-reads `NSScreen`, and reports `WindowChange::Screens(Vec<Monitor>)`; `Windows::record` replaces `screens` wholesale, and `monitor_for` picks the monitor a frame is on, falling back to the first. A display going off is a `Screens` event with one fewer monitor in it, and the windows macOS relocated arrive as `Moved` events from the accessibility observers that were already watching them. Placement after the change uses the new rectangles because it reads them out of the state.

So the missing piece is not the event. It is that a `Monitor` is two rectangles and nothing else, and a rule about the built-in display cannot be stated about a rectangle.

## Change 1: a monitor has an identity

`crates/freddie_windows/src/lib.rs`, before:

```rust
/// full frame, for locating a window; visible frame = full minus menu bar and dock.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Monitor {
    pub full: Frame,
    pub visible: Frame,
}
```

after:

```rust
/// The display's id as macOS assigns it. Not stable across a reboot or a reconnect, which is
/// what makes it right for "the display in this arrangement" and wrong for anything remembered.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DisplayId(pub CGDirectDisplayID);

/// full frame, for locating a window; visible frame = full minus menu bar and dock.
#[derive(Clone, PartialEq, Debug)]
pub struct Monitor {
    pub full: Frame,
    pub visible: Frame,
    pub id: DisplayId,
    /// The laptop's own display. `CGDisplayIsBuiltin`, not a guess from the name or the size.
    pub builtin: bool,
    /// What the display calls itself: `NSScreen::localizedName`. This is also what
    /// `betterdisplaycli` addresses a display by.
    pub name: String,
}
```

`Monitor` stops being `Copy`, because of the name. `Windows::monitor_for` returns one, so it clones:

`crates/mercury/src/state/mod.rs`, before:

```rust
    pub fn monitor_for(&self, frame: Frame) -> Option<Monitor> {
        self.screens
            .iter()
            .find(|m| m.full.contains(frame.x, frame.y))
            .or_else(|| self.screens.first())
            .copied()
    }
```

after:

```rust
    pub fn monitor_for(&self, frame: Frame) -> Option<Monitor> {
        self.screens
            .iter()
            .find(|m| m.full.contains(frame.x, frame.y))
            .or_else(|| self.screens.first())
            .cloned()
    }
```

`read_monitors` fills the new fields. The display id is `NSScreen`'s device description under `NSScreenNumber`; `CGDisplay::new(id).is_builtin()` is `CGDisplayIsBuiltin`, declared in `core-graphics 0.25` at `src/display.rs:460`, which the crate already depends on.

`crates/freddie_windows/src/lib.rs`, before:

```rust
    screens
        .iter()
        .map(|screen| Monitor {
            full: to_ax(screen.frame()),
            visible: to_ax(screen.visibleFrame()),
        })
        .collect()
```

after:

```rust
    screens
        .iter()
        .map(|screen| {
            let id = display_id(&screen);
            Monitor {
                full: to_ax(screen.frame()),
                visible: to_ax(screen.visibleFrame()),
                builtin: id.is_some_and(|id| CGDisplay::new(id.0).is_builtin()),
                id: id.unwrap_or(DisplayId(0)),
                name: screen.localizedName().to_string(),
            }
        })
        .collect()
```

with:

```rust
/// The `CGDirectDisplayID` behind an `NSScreen`, out of its device description.
///
/// `None` for a screen whose description does not carry one, which is not something a real display
/// does; such a screen keeps its rectangles and is never the built-in, so placement still works on
/// it and no rule ever names it.
fn display_id(screen: &NSScreen) -> Option<DisplayId> {
    let key = NSString::from_str("NSScreenNumber");
    let value = unsafe { screen.deviceDescription() }.objectForKey(&key)?;
    let number: Retained<NSNumber> = value.downcast().ok()?;
    Some(DisplayId(number.as_u32()))
}
```

The tests in `crates/mercury/tests/transitions.rs` build `Monitor` literals; they gain the three fields, with a helper so the fixtures stay readable:

```rust
/// A monitor at `full`, whose visible frame is the same, external and unnamed. What a test that
/// only cares about rectangles wants.
fn monitor(full: Frame) -> Monitor {
    Monitor {
        full,
        visible: full,
        id: DisplayId(1),
        builtin: false,
        name: "Test Display".to_owned(),
    }
}
```

## Change 2: the rule

A handler on `Windowed` already records the change. The rule sits beside that record, reads the set that just arrived, and asks for the command.

It is a function of the arrangement and nothing else, which is what makes it safe to fire on every screen change:

- The built-in is present and so is at least one external: turn the built-in off.
- No external is present: turn the built-in on. It fires when the last external is unplugged, and it is what stops a machine from being left with nothing lit if macOS does not bring the built-in back on its own. Turning on a display that is already on is a command that succeeds and changes nothing.
- The built-in is absent and an external is present: this is the docked steady state, and there is nothing to do.

Once the built-in is off, the first case stops matching, because the built-in is no longer in the set. That is what keeps the rule from running on its own output.

The membership guard is the second half of it: the arrangement changes for a resolution change and a rearrangement too, and neither should run a subprocess. So the handler compares the ids it is about to record against the ids it holds, and does nothing when the set of displays is the same.

`crates/mercury/src/handlers/window.rs`, before:

```rust
pub(crate) fn record_windows(ev: &WindowEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    root.windows.record(&ev.change);
    Vec::new()
}
```

after:

```rust
pub(crate) fn record_windows(ev: &WindowEvent, node: Node<&mut Mercury, ()>) -> Vec<MercuryEffect> {
    let root = node.parent;
    let effects = match &ev.change {
        WindowChange::Screens(screens) if root.windows.displays_changed(screens) => {
            builtin_command(screens).into_iter().collect()
        }
        _ => Vec::new(),
    };
    root.windows.record(&ev.change);
    effects
}

/// The command that puts the built-in display where the arrangement says it should be, or `None`
/// when it is already there.
fn builtin_command(screens: &[Monitor]) -> Option<MercuryEffect> {
    let builtin = screens.iter().find(|m| m.builtin);
    let external = screens.iter().any(|m| !m.builtin);
    let connected = match (builtin, external) {
        (Some(_), true) => false,
        (_, false) => true,
        (None, true) => return None,
    };
    Some(MercuryEffect::Run(Run {
        program: "betterdisplaycli".to_owned(),
        args: vec![
            "set".to_owned(),
            "--name=Built-in".to_owned(),
            format!("--connected={}", if connected { "on" } else { "off" }),
        ],
        cwd: PathBuf::from("/"),
    }))
}
```

and on `Windows`:

```rust
    /// Whether `screens` is a different set of displays than the one held, by id. A display that
    /// changed resolution or moved in the arrangement is the same display, and this is false for it.
    pub(crate) fn displays_changed(&self, screens: &[Monitor]) -> bool {
        let ids = |monitors: &[Monitor]| {
            monitors.iter().map(|m| m.id).collect::<HashSet<_>>()
        };
        ids(&self.screens) != ids(screens)
    }
```

The command runs through `MercuryEffect::Run` from `run-effect.md`. `cwd` is `/`, because the command does not care and the payload has to say something; the effect side has no default to fall back on.

`--name=Built-in` addresses the display by the name BetterDisplay shows it under, which is what its CLI matches on. The name macOS reports for the same display is in `Monitor::name`, and a machine where the two disagree is a machine where this rule names the wrong display; the log shows what `betterdisplaycli` said it did.

## What has to be in place

`betterdisplaycli` is a separate binary from the app, and it is not installed on this machine:

```
brew install waydabber/betterdisplay/betterdisplaycli
```

BetterDisplay itself has to be running, and connection management is a Pro feature. Nothing here degrades gracefully without them: `betterdisplaycli` not on `PATH` is a spawn failure in the log and no display changes.

## What is not measured

Two things, and both are behavior of the machine rather than of this code.

Unplugging the only external while the built-in is off: whether macOS restores the built-in itself, and what `NSScreen.screens` reports in the moment between. The rule's second case fires on that transition either way, so the outcome is the same, but it is the case to watch the log through the first time.

Whether the screen-parameters notification arrives while the arrangement is settling, which would mean two `Screens` events for one plug. The membership guard makes the second one a no-op if the ids match, and a genuine second command if they do not.

## Tests

The rule is a pure function of the screen set, so it is a table:

```rust
#[test]
fn the_builtin_follows_the_externals() {
    let builtin = Monitor { builtin: true, name: "Built-in".to_owned(), ..monitor(SCREEN_FRAME) };
    let external = Monitor { id: DisplayId(2), ..monitor(OTHER_FRAME) };

    // Docked with the lid open: turn the built-in off.
    assert_eq!(
        builtin_command(&[builtin.clone(), external.clone()]),
        Some(betterdisplay("off"))
    );
    // Docked, built-in already off: nothing to do.
    assert_eq!(builtin_command(&[external.clone()]), None);
    // Undocked: the built-in is on, whether or not it already was.
    assert_eq!(builtin_command(&[builtin.clone()]), Some(betterdisplay("on")));
    assert_eq!(builtin_command(&[]), Some(betterdisplay("on")));
}
```

and the guard is a transition test: two `Screens` events carrying the same ids and different rectangles produce a command for the first and nothing for the second.
