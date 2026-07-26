# screenshots

Taking a screenshot from a binding, landing it at a known path, and knowing in state where it landed, so the next binding can hand it to an agent (send-to-agent.md).

## The channels

The OS has one good tool and mercury already knows how to run programs.

`screencapture` is the whole capture surface as a CLI: `-i` for the system's interactive drag-a-region UI (space toggles to window-picking), `-l <CGWindowID>` for a specific window with no interaction, `-R x,y,w,h` for an exact rect, `-o` to drop the window shadow, `-x` to mute the shutter sound. It writes a file, or `-c` puts the image on the clipboard. mercury already holds real `CGWindowID`s in the model (`freddie_windows` reports them), so "screenshot the focused window" needs no interaction and no guessing: the id is in state.

The process that invokes `screencapture` needs the Screen Recording TCC grant, which mercury does not hold today. That is a new permission prompt, once.

Chrome can also capture its own tab over the socket (`chrome.tabs.captureVisibleTab` needs `<all_urls>`; full-page needs `chrome.debugger`), but the image comes back as base64 in a frame, and the socket caps frames at 64 KB. The OS path captures the same pixels for the visible case with no permission growth on the extension and no frame-size question, so tab capture through the extension is deferred until full-page capture (content below the fold) is actually wanted. chrome-control.md places this.

## The effect and the event

Capture is slow (interactive capture is unboundedly slow: the user is dragging) and its product is a fact the model wants, so it is the standard shape: a fire-and-forget effect on its own thread, completion arriving as an event.

```rust
/// What to capture. The payload carries everything: the window variant holds the id because the
/// handler has it in state, and the performer looks nothing up.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum ScreenshotTarget {
    /// The system's interactive picker: drag a region, or space to pick a window.
    Interactive,
    /// A specific window, no interaction. `-o -l <id>`.
    Window(WindowId),
}

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct ScreenshotEffect {
    pub target: ScreenshotTarget,
    /// Absolute. The handler computed it (directory plus timestamp); the performer expands nothing.
    pub path: PathBuf,
}
```

`MercuryEffect::Screenshot(ScreenshotEffect)`. The performer spawns a thread, runs `screencapture -x [target flags] <path>`, and on a zero exit with the file present sends the event; a cancelled interactive capture (exit 1, no file) logs at debug and sends nothing.

```rust
#[derive(Debug)]
pub struct ScreenshotTaken {
    pub path: PathBuf,
}
```

`MercuryEvent::Screenshot(ScreenshotTaken)`, and the handler assigns it: `root.last_screenshot = Some(event.path.clone())`. Assignment, not accumulation, per the idempotence rule; the previous screenshot's path is overwritten, the file stays on disk.

The directory and name are the handler's decision at dispatch time: `<dir>/mercury-<YYYYMMDD-HHMMSS>.png`. Which directory is an open question below.

## Bindings

A capture family, probably on home or nav; the keys are unassigned until the keymap has room. The two that matter:

- Interactive capture: one decision, then you are done choosing; but what usually follows is sending it somewhere, so ending in home with `last_screenshot` set is the default, and send-to-agent.md's send binding is the natural next press.
- Focused-window capture: reads the focused window's id out of state, no interaction, same landing.

Clipboard is not part of the effect: a screenshot that should also be on the clipboard is `Copy(Copied::Text(path))` composed after the event lands, or a later `Copied` variant that carries image data if pasting pixels is ever wanted.

## Open questions

- Which directory. `~/Screenshots`, the Desktop (the OS default), or somewhere the agents already look. This decides itself when send-to-agent.md decides how agents receive paths.
- Whether the shutter sound stays muted (`-x`) always, or only for the window variant.
- Whether `last_screenshot` and the last download (chrome-control.md, `chrome.downloads`) are one field or two. send-to-agent.md owns that decision.
