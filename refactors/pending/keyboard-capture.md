# capturing keyboard events

v1 grabs the keyboard on macOS, swallows every key, dispatches it, and emits whatever the model produces. Nothing reaches the app except through that emit, so typing and remapping take the same route and stay in order.

## The crates

`freddie_keys` is the platform-neutral vocabulary: the `Keyboard` enum, a `KeyEvent`, and the `EventTrigger` impl, so mercury binds `Keyboard::KeyR` directly.

`freddie_keyboard` is the platform layer: an OS-agnostic API (`run`, `emit`) over `Keyboard`, with the OS-specific work behind it. v1 ships one backend, macOS on `core-graphics` (Quartz Event Services). Linux (X11/XTEST or evdev/uinput) and Windows (`SetWindowsHookEx` + `SendInput`) are separate backends behind `cfg`, whenever we want them. core-graphics is macOS-only, so it stays inside that backend and never leaks up. Each backend owns its own native keycode table, since a `CGKeyCode` is nothing like an X11 keysym or a Windows virtual-key. core-graphics' wrappers are safe, so the macOS backend needs no unsafe.

mercury writes the loop on top of the OS-agnostic API: spawn the capture thread, pick the swallow and exit policy, forward keys into its channel.

## The macOS tap

`CGEventTap::new` over `[KeyDown, KeyUp]`, with a callback that returns `Keep`, `Drop`, or `Replace`. For a real key the callback reads the keycode, maps it to a `Keyboard`, sends it to the event channel, and returns `Drop`, so the key never reaches the app. For a key we emitted ourselves the callback returns `Keep` and does not dispatch it. The tap runs on its own thread: build a run-loop source from its mach port, add it to the thread's run loop, enable, run. If macOS disables the tap for being slow it delivers a `TapDisabled` event, so the callback watches for that and re-enables. Keep the callback to a field read, a lookup, and a send.

## Telling our own keys apart

An emitted key comes back to our own tap, and we must not re-dispatch it. The macOS backend keeps one private `CGEventSource` and stamps every emitted event's `USER_DATA` field with a magic number. The tap reads that field; if it is ours, `Keep` and move on. This replaces the old `SYNTHETIC` counter, which assumed the next two callbacks were our press and release and corrupted real input the moment a key landed in between.

## Emitting

`emit(key)` builds a keyboard event from the private source, stamps `USER_DATA`, and posts it. A chord like cmd+r is a press-and-release sequence with the modifier held around the key, either as `emit_chord(mods, key)` or by setting the event flags. The effect loop is the only caller, so emits leave one at a time in the order the model produced them.

## Escape and stopping

Escape calls `process::exit(0)` from the callback, before anything else. It is the bail-out, and it does not depend on the model or the channel, so a wedged model cannot trap the keyboard. Killing the process frees the tap either way, since the WindowServer drops taps for dead processes, `kill -9` included. Dropping the `CGEventTap` or stopping its run loop frees it too, which is how a `Capture` handle would stop cleanly, but v1 does not need one: exit is the stop.

## What's left to build

mercury today binds `Keyboard` through a `Key(pub Keyboard)` newtype, carries `Type(Keyboard)` / `Command(Keyboard)` effects, and runs `freddie_keyboard::listen` (watching, not grabbing), printing effects instead of emitting. To get to a real v1:

1. Write `freddie_keys`: the `Keyboard` enum (letters, digits, `Space`/`Return`/`Tab`/`Escape`, arrows, modifiers, F1-F24), a `KeyEvent { key, down }`, and `impl EventTrigger for Keyboard`. Depends on `bind`.

2. Build the macOS backend in `freddie_keyboard`: the tap above, `emit` and `emit_chord` with the `USER_DATA` stamp, and the `CGKeyCode` to `Keyboard` table both ways. The F21-F24 codes depend on the keyboard, so grab whatever the key actually sends. This is where core-graphics replaces the current rdev backend.

3. Drop mercury's wrapper: bind `Keyboard` variants straight (`MercuryTrigger::Key(Keyboard)`), delete the newtype. `AnyKey` stays.

4. Wire mercury live: `main.rs` runs `run` instead of `listen`, exits on escape, forwards each `KeyDown`. The effect loop emits instead of printing, `Type(k)` emits `k` and `Command(k)` emits cmd+`k`. Keep the killswitch timer as a backstop.

5. Test on the machine: Accessibility granted, escape and `kill -9` both free the keyboard, the tag stops the echo, an F-key above F12 maps.

## Two things to decide

Escape is doing two jobs. mercury's model binds it (`to_home`, `passthru`), but v1 wants it as the panic exit, so those binds will not fire while we grab. For v1, let escape be the bail-out and drive layers with other keys; hand it back to the model later, once a different panic key or the stop handle covers us.

Unbound keys get dropped. Swallow-all means a key the layer does not bind returns `None`, produces no effect, and never reaches the app. In home that is every non-command key. Fine for a modal remapper, but it is a choice: a layer that should pass its extra keys needs an explicit passthrough (passthrough.md).

## Permissions

An active tap needs Accessibility plus Input Monitoring, granted to whatever launches the binary. First run prompts; flip it in System Settings and relaunch.
