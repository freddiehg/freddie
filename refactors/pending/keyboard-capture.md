# capturing keyboard events

Getting every key the runner cares about into its event channel, and getting back out cleanly. `rdev` does the OS-level work; `freddie_keyboard` wraps it into the shared primitives; the consumer (mercury, figaro) writes the capture loop. This is that split, the actual code, and where the sharp edges are.

## Crate organization

Three layers, from the OS up:

- `rdev` is the existing cross-platform library that talks to the OS: a `CGEventTap` on macOS, low-level hooks elsewhere. It is the "library that already does the work," and nothing above it touches the platform directly.
- `freddie_keyboard` is a thin shared crate over rdev. rdev alone is not enough to inline in each app: it hands back raw `rdev::Key` codes and does not tag the keys we emit ourselves, so every consumer would re-write the same name mapping and self-feedback guard. mercury and figaro both need those, so they live here once. This crate owns the whole platform surface (rdev is a private dependency) and exposes `run`, `listen`, `emit`, `name`, `KeyEvent`, `Error`.
- The consumer wires the capture loop: spawn the source thread, decide the swallow/exit policy, map keys onto its own event type, and forward into its channel. That is app-specific (the channel type, the escape policy, the key-to-event mapping differ per app), so it is not in the crate.

## The freddie_keyboard API

What the crate exposes today:

```rust
pub use rdev::Key;

pub struct KeyEvent { pub key: Key, pub press: bool } // press=false is key-up

pub fn run(on_key: impl Fn(KeyEvent) + 'static) -> Result<(), Error>;    // grab: swallow every key, hand it to on_key
pub fn listen(on_key: impl Fn(KeyEvent) + 'static) -> Result<(), Error>; // observe: swallow nothing
pub fn emit(key: Key) -> Result<(), Error>;                              // press+release, guarded so run ignores it
pub const fn name(key: Key) -> Option<&'static str>;                     // "a", "space", "escape", ... or None
pub enum Error { Grab(..), Listen(..), Simulate(..) }
```

`run` and `listen` both block the calling thread (rdev owns the run loop there), so the consumer runs them on their own `std::thread`. `run` swallows everything and re-emits are the consumer's job via `emit`; `emit` bumps a `SYNTHETIC` counter that `run`'s callback decrements so our own output is not fed back in.

## v1 in mercury: swallow all, escape exits

v1 grabs (`run`), swallows every key, and hardcodes one exception: escape exits the process. No per-key decision, no shared state. The whole capture source:

```rust
fn spawn_keyboard_source(event_tx: UnboundedSender<MercuryEvent>) {
    std::thread::spawn(move || {
        let grabbed = freddie_keyboard::run(move |ev| {
            if ev.key == freddie_keyboard::Key::Escape {
                std::process::exit(0); // the one way out of a full hijack
            }
            if !ev.press {
                return; // v1 acts on key-down; key-up (ev.press == false) is available but unused
            }
            if let Some(name) = freddie_keyboard::name(ev.key) {
                let _ = event_tx.send(key(name));
            }
        });
        if let Err(e) = grabbed {
            eprintln!("keyboard: {e}"); // usually Accessibility is not granted
        }
    });
}
```

Everything else is unchanged from the current main.rs: the event loop dispatches each `key(name)`, the effect loop prints, the timed killswitch stays as a backstop. The only edit to move v1 from observe to hijack is `listen` becoming `run` plus the escape branch.

Escape is a callback-level hard exit, not a normal bind, on purpose. If the model wedges (a bad transition, a panic in a handler, a full channel), a bind routed through dispatch could stop firing, and a fully hijacked keyboard would then have no way out. The callback escape does not depend on dispatch running at all, so it always works. (Swallow-versus-pass for escape is moot: we exit before it matters.)

## Exit tears down the capture

The safety of v1 rests on one fact: killing the process removes the tap. A `CGEventTap` is owned by the process that created it and registered on that process's run loop; the WindowServer drops it when the process dies, including a hard kill (SIGKILL) or a crash. So `process::exit` on escape, the timed `_exit` killswitch (event-loop.md), and a panic all restore the keyboard. There is no persistent OS state to clean up and no way to leave the keyboard captured by a dead process.

This is by design, but it has to be confirmed live: run mercury grabbing, press escape, check the keyboard is normal; then repeat with `kill -9` on the process to confirm a hard kill also frees it.

## Press, release, repeat

- v1 dispatches on `KeyPress`. `KeyRelease` arrives as `ev.press == false` and is available for anything that later needs hold or release (modifiers as layers, see modifier-keys.md), but v1 ignores it.
- v1 has no repeat feature. It does nothing to synthesize repeats and nothing special with the OS's own repeated `KeyPress`.
- Post-v1: if a held key should re-fire its output at a chosen rate, that is a "send an event every X ms while the key is held" timer loop in the consumer, started on `KeyPress` and stopped on `KeyRelease`, not something read off OS auto-repeat. Deliberately a consumer concern, not the crate's.

## v2: swallow only what's bound

Swallowing everything means every passed-through key is a channel round-trip plus a re-emit (keyboard-emit.md). The optimization is to swallow only the keys the active state binds and pass the rest natively (passthrough.md), so passthrough costs a set lookup, not a re-emit. That requires `run` to let the callback return `Some` (pass) or `None` (swallow) per key, which it does not yet expose.

To decide per key, the callback needs the active trigger set:

- `bind::accumulate` computes the set from the current state.
- The dispatch loop publishes it to the callback through a shared cell it updates on every state change (`arc_swap::ArcSwap<HashSet<Trigger>>`, or `Arc<Mutex<_>>`). The callback loads and looks up. This is the only reason ArcSwap enters the picture.
- Bound key: swallow, send to the channel; dispatch's effects (a remap, or an explicit `Passthru` re-emit) come out the effect side.
- Unbound key: pass natively; it never enters the loop.

The swallow decision stays synchronous in the callback while dispatch stays async. The one subtlety is staleness: the set the callback sees lags the true state by one update if a key lands mid-transition. For a single keyboard that window is negligible, which is why the set is published atomically rather than locked.

## Events rdev delivers

`rdev::Event { time, name, event_type }`. We care about `event_type`:

- `KeyPress(Key)` / `KeyRelease(Key)` for every key, including modifiers (`ShiftLeft`, `MetaLeft` = cmd, `Alt`, `ControlLeft`).
- rdev does not surface a combined modifier-flags value; modifier state is tracked from the modifier keys' own press/release.

`rdev::Key` is a named enum (`KeyA`..`KeyZ`, `Num0`..`Num9`, `Space`, `Escape`, `Return`, `Tab`, arrows, function keys, modifiers). `freddie_keyboard::name` maps the common ones to a stable lowercase string so the model binds by name, not key code. Anything not in the map returns `None` and is dropped; the map has to grow to whatever the bindings use (digits, punctuation, arrows, modifiers). `Event.name` also carries the OS's layout-dependent character, which we currently ignore in favor of the layout-independent `Key`.

## Threading and the channel

`run` blocks on a `CFRunLoop` on its thread. The callback forwards each non-escape key-down into the runner's event channel with a `tokio::mpsc::UnboundedSender` (cloneable, `Send`, callable from the rdev thread), and calls `process::exit` on escape. Keep the callback to a check, a send, and a return: macOS disables a tap whose callback is slow (~1s), and a blocked callback drops input.

## Permissions (macOS)

- v1 grabs, so it needs Accessibility, plus Input Monitoring for the keyboard.
- `listen` alone would need only Input Monitoring.

Granted to whatever launches the binary (the terminal in dev, the built `.app` later). First run prompts; toggle in System Settings > Privacy & Security and relaunch. The exact TCC service shifts by macOS version, so verify on the target.

## Footguns

- Self-lockout: grabbing with no way out. v1's escape exit is the primary safety because it does not depend on dispatch; the timed killswitches (event-loop.md) back it up (`Kill` on a 5s timer, hard `_exit` at 10s, `SIGHUP` fatal).
- Tap timeout: macOS disables a slow tap and sends a disabled event. Whether rdev re-enables it automatically or `freddie_keyboard` must is open (verify). Keep the callback trivial regardless.
- Secure input: password fields, and apps using the secure-input API, bypass the tap; those keys cannot be captured.

## Open questions

- Confirm live that `process::exit` on escape, and a `kill -9`, both free the keyboard.
- Does rdev re-enable the tap after a macOS timeout, or must we?
- How wide does `name` need to be, and do we key off `Key` (layout-independent) or `Event.name` (the typed character)?
- Is rdev's `grab` reliable enough, or does real swallowing need raw `core-graphics`?
