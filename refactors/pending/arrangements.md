# window arrangements

A named arrangement puts several windows where they belong in one binding, where placement today moves only the focused window. The first arrangement wanted is a recording stage: QuickTime Player set up for a good screen recording, with the window being recorded framed cleanly.

## What an arrangement is

Placement machinery exists for one window: the model knows the focused window's id and frame, and `SetFrame` moves it. An arrangement extends that across windows and apps, which drags in the one genuinely new problem: foregrounding is asynchronous. `Foreground(App)` asks, and the `Foregrounded` event confirms, so an arrangement that touches two apps cannot be a flat `Vec<MercuryEffect>`; it is a state with a cursor, advancing one step per confirmation, the same waiting shape as the dictation gesture in send-to-agent.md.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Foreground the app; the arrangement advances on its Foregrounded event.
    Foreground(App),
    /// Place the now-focused window; advances immediately.
    Place(WindowFrame),
    /// Send the now-focused app a chord; advances immediately.
    Tap(Chord),
}

/// In-flight arrangement state: the steps and how far along we are. Lives on the layer that
/// started it; leaving the layer abandons the remainder rather than firing it into whatever
/// is frontmost later.
#[derive(Debug)]
pub struct Arranging {
    pub steps: &'static [Step],
    pub next: usize,
}
```

An arrangement is a `&'static [Step]` named in code, like overlays are; there is no config file. Each binding points at one.

## The recording stage

The concrete arrangement: QuickTime Player recording a clean 16:9 region with the recorded window filling it.

```rust
const RECORDING_STAGE: &[Step] = &[
    // The window being recorded, centered in a 16:9 region of the main display,
    // margins clear of the dock and menu bar.
    Step::Foreground(App::Chrome),
    Step::Place(RECORDING_FRAME),
    // QuickTime, then File > New Screen Recording (ctrl-cmd-n), which raises the
    // system capture toolbar ready to select that region.
    Step::Foreground(App::QuickTimePlayer),
    Step::Tap(Chord { key: Key::KeyN, flags: ModifierFlags::CONTROL | ModifierFlags::COMMAND }),
];
```

This needs `App::QuickTimePlayer` (`com.apple.QuickTimePlayerX`) as a new `App` variant with the usual round-trip through `from_bundle_id`.

What the last step cannot do is drag the capture region: the system recording toolbar's region selection is mouse-driven UI with no API. Two outs, in preference order: record the entire display and let the 16:9 frame be the composition (crop in post or do not crop at all, since the stage frame is what viewers see anyway), or finish the region drag by hand once, since macOS remembers the last-used capture region between recordings. mouse-mode.md eventually makes even the drag scriptable, but the arrangement does not wait for it.

The recorded app is part of the arrangement's name, not a parameter: `RECORDING_STAGE` stages Chrome, and a Ghostty recording stage is a second constant when it is wanted. Parameterizing over "whatever is frontmost" is a smaller change later than a wrong guess now.

`RECORDING_FRAME`'s numbers (which display, exact rect) depend on the monitor layout the model already knows (`freddie_displays`), and the right 16:9 rect is the user's call at implementation time.

## Where it leaves you

Starting an arrangement is one decision, and what follows is using the stage, so the binding goes home when the last step confirms. While `Arranging` is live the layer shows it in the overlay; keys other than escape are inert rather than queued behind it. Escape abandons the remaining steps, moves nothing back, and goes home.

## Open questions

- Whether `Place` should address a window explicitly (app plus window id from state) instead of "the now-focused window", which would survive an app whose focused window is not the one meant. The focused-window version is v1; the addressed version is what window-restore-style layouts would need anyway.
- Entire-display recording versus the remembered region, per above.
- Which key starts it, and whether arrangements get their own chooser layer once there are three of them.
