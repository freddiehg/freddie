# capturing keyboard events

Getting every key the active state cares about into the runner's event channel, and letting the rest through untouched. `freddie_keyboard` (over `rdev`) is the source. This is what it has to do and where the sharp edges are.

## The two modes

`rdev` gives two ways to see the keyboard, and `freddie_keyboard` wraps both:

- `listen(cb)` observes without swallowing. Every key still reaches the focused app; a copy goes to `cb`. This is v1, safe to run because it changes nothing. On macOS it is a listen-only `CGEventTap` and needs Input Monitoring.
- `run(cb)` grabs: the callback returns `Some(event)` to pass a key or `None` to swallow it. This is the real remapper. On macOS it is an active `CGEventTap` and needs Accessibility (plus Input Monitoring). Behind rdev's `unstable_grab` feature.

Both block the calling thread and must run on their own thread (rdev owns the run loop there).

## Events rdev delivers

`rdev::Event { time, name, event_type }`. We care about `event_type`:

- `KeyPress(Key)` / `KeyRelease(Key)` for every key, including modifiers (`ShiftLeft`, `MetaLeft` = cmd, `Alt`, `ControlLeft`).
- rdev does not surface a combined modifier-flags value; modifier state is tracked from the modifier keys' own press/release.

`rdev::Key` is a named enum (`KeyA`..`KeyZ`, `Num0`..`Num9`, `Space`, `Escape`, `Return`, `Tab`, arrows, function keys, modifiers). `freddie_keyboard::name(Key)` maps the common ones to a stable lowercase string so the model binds by name, not key code. Anything not in the map is currently dropped; the map has to grow to whatever the bindings use (digits, punctuation, arrows, modifiers). `Event.name` also carries the OS's layout-dependent character, which we currently ignore in favor of the layout-independent `Key`.

## Press, release, repeat

- v1 dispatches on `KeyPress` only and ignores `KeyRelease`. Fine for triggers (a key acts on the way down), not for anything that cares about hold or release (modifiers as layers, see modifier-keys.md).
- Held keys auto-repeat: the OS sends repeated `KeyPress` with no intervening `KeyRelease`. So repeat is free: handle each repeated `KeyPress` (a passthrough re-emits, a remap emits its output again) and it repeats at the native cadence, no timer needed. Deduping (tracking which keys are down and ignoring repeats) is the opposite choice, for a key that should fire once on the way down (a layer switch); that is userland's, in the consumer, not the crate. A self-driven repeat timer earns its place only if you want a rate different from the OS's, which is niche.

## Selective swallow (the real capture)

v1 swallows nothing. The remapper should swallow only the keys the active state binds and pass the rest natively (passthrough.md), so passthrough costs a set lookup, not a re-emit.

Mechanics:

- The `grab` callback decides `Some`/`None` synchronously, so it needs the active trigger set. `bind::accumulate` computes it from the current state. Publish it to the callback through a shared cell the dispatch loop updates on every state change (`arc_swap::ArcSwap<HashSet<Trigger>>`, or `Arc<Mutex<_>>`); the callback loads and looks up.
- Bound key: return `None` (swallow) and send the event to the channel. Dispatch runs async and its effects (a remap, or an explicit `Passthru` re-emit) come out the effect side.
- Unbound key: return `Some(event)` (pass natively). It never enters the loop.

So the swallow decision is synchronous in the callback while dispatch stays async. The subtlety is staleness: the set the callback sees lags the true state by one update if a key lands mid-transition. For a single keyboard that window is negligible, but it is why the set is published atomically.

## Threading and the channel

`listen`/`run` block on a `CFRunLoop` on their thread. The callback forwards into the runner's event channel with a `tokio::mpsc::UnboundedSender` (cloneable, `Send`, callable from the rdev thread). Keep the callback to a lookup, a send, and a return: macOS disables a tap whose callback is slow (~1s), and a blocked callback drops input.

## Permissions (macOS)

- `listen`: Input Monitoring.
- `run` and any emit: Accessibility, plus Input Monitoring for the keyboard.

Granted to whatever launches the binary (the terminal in dev, the built `.app` later). First run prompts; toggle in System Settings > Privacy & Security and relaunch. The exact TCC service shifts by macOS version, so verify on the target.

## Footguns

- Self-lockout: grabbing with no way out. The dev killswitches (event-loop.md) are the safety: `Kill` on a 5s timer, hard `_exit` at 10s, `SIGHUP` fatal.
- Tap timeout: macOS disables a slow tap and sends a disabled event. Whether rdev re-enables it automatically or `freddie_keyboard` must is open (verify). Keep the callback trivial regardless.
- Secure input: password fields, and apps using the secure-input API, bypass the tap; those keys cannot be captured.

## Open questions

- Does rdev re-enable the tap after a macOS timeout, or must we?
- How wide does `name` need to be, and do we key off `Key` (layout-independent) or `Event.name` (the typed character)?
- Publishing the active trigger set to the callback: `ArcSwap` vs `Mutex`, and the one-update staleness.
- Is rdev's `grab` reliable enough, or does real swallowing need raw `core-graphics`?
