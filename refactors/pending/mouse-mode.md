# mouse mode

Driving the pointer from the keyboard: moving it, clicking, dragging, and scrolling, as a layer in the model. voicemode had all of it (directional scroll, hover mode, six click variants, Homerow labels, a grid layer, warpd), so this is a replacement, not an invention, and it comes in stages.

## Stage one: move, click, scroll

A `MouseLayer` in the state tree, entered from home, staying put while you work the pointer, `escape` home like everywhere else.

The platform half is a new crate, `freddie_pointer`, shaped per docs/platform-apis.md: it wraps `CGEventCreateMouseEvent` and `CGEventCreateScrollWheelEvent`, posts through the session tap location like the key emitter, and reuses one `CGEventSource` for the same shm reason `Emitter` does (refactors/past/cgeventsource-shm-leak.md). It rides the Accessibility grant mercury already holds.

The effects:

```rust
#[cfg_attr(feature = "testing", derive(PartialEq))]
#[derive(Debug)]
pub struct Delta {
    pub dx: f64,
    pub dy: f64,
}

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct Click {
    pub button: MouseButton,
    /// 1 for a click, 2 for a double click; rides `kCGMouseEventClickState`.
    pub count: u8,
}

#[cfg_attr(feature = "testing", derive(PartialEq))]
#[derive(Debug)]
pub enum PointerEffect {
    /// Move the pointer by a delta from where it is.
    MoveBy(Delta),
    /// Press and release at the current position.
    Click(Click),
    /// Press without releasing: the start of a drag. Movement while held drags.
    Press(MouseButton),
    Release(MouseButton),
    /// Scroll wheel lines at the current position.
    Scroll(Delta),
}
```

`MercuryEffect::Pointer(PointerEffect)`, performed inline on the effect loop (posting a CGEvent is as cheap as `Tap`).

`MoveBy` and `Scroll` are relative, and the OS owns the pointer's absolute position, so the performer reads the current location (`CGEventGetLocation` on a fresh null-source event) to compute the target point. That is a performer reading the outside world, which the architecture forbids in general; the alternative is mirroring the pointer into state through a pointer-moved event stream, which fires at input rate for every physical mouse twitch and would be noise the model never reads except here. This deviation needs sign-off before it goes further; the copy fallback's osascript read is the precedent.

## Continuous motion

Holding `j` should glide, not step. The model holds which direction keys are down and a repeating timer drives motion while any is held:

```rust
pub struct MouseLayer {
    /// Direction keys currently held. Insert on down, remove on up.
    pub held: BTreeSet<Direction>,
    /// Ticks since motion began, for acceleration.
    pub ticks: u32,
}
```

Key down inserts the direction and, if `held` was empty, starts a repeating timer (the timer machinery exists; timer creation is `state.handle`'s one sanctioned impurity). Each `TimerFired` dispatches through the model: the handler reads `held`, computes a delta with acceleration (speed as a function of `ticks`, capped), and returns `Pointer(MoveBy(delta))`. The last key up clears `held` and cancels the timer. This is not a poll: the timer exists only while a key is physically held, and dies with the keyup. The tick rate and the acceleration curve are tuning numbers to settle by feel; voicemode ran around 20 Hz and felt fine.

Scroll works identically with its own direction keys and `Scroll` instead of `MoveBy`.

## Stage two: the grid

Bisection targeting, warpd's `g` mode: the overlay draws a grid over the screen, each key halves the region, a few presses put the pointer anywhere. It is a state (`region: Frame` narrowing per press) plus the overlay (which already draws) plus one final `MoveBy` computed as absolute-minus-current. No new OS surface at all; this is the highest-leverage stage after basic movement.

## Stage three: labels

Homerow-style hints: enumerate clickable elements, label them, jump by typing a label. Two sources, both already described in ideas.md: the AX tree (role and title, brittle where apps label nothing) and Vision OCR of the screen (works everywhere, including apps with no AX story). Both want the overlay for label drawing. This stage is deliberately unplanned here; it gets its own doc when stage one and two are live.

## Bindings and where they leave you

Movement, clicking, and scrolling all repeat, so the layer stays through all of them. A click that ends the errand is the common case though, so alongside `click` there is a click-and-go-home (and a click-then-typing for clicking into a text field, which would otherwise strand the next words in a command layer). Keys are unassigned until the keymap has room; the layer entry key and the direction cluster (hjkl against the Kinesis layout) are the user's call.

## Open questions

- The `MoveBy` position read in the performer, per above: acceptable deviation, or does the pointer become model state fed by an event stream after all.
- Tick rate and acceleration curve, to be tuned live.
- Whether drag needs modifier-held variants (shift-drag, cmd-drag) as first-class effects or as `Press` composed with the flags the emitter already sends on keys.
