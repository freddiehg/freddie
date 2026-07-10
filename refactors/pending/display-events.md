# display events

We want an event when an external monitor is connected or disconnected, fed into the runner as an input alongside keys and foreground changes.

A note, not a plan. Nothing here is measured.

## Shape

Another source, exactly like foreground-events.md. Something registers with the main run loop, its callback does one `send` into the event channel and returns, and mercury dispatches the event on the worker thread. `freddie_main_loop` already gives us the main thread, so the prerequisite is done.

It is a `freddie_*` crate, not mercury's, by the rule in the README: figaro would write it identically.

## Two candidate APIs

`CGDisplayRegisterReconfigurationCallback`, in Core Graphics. `core-graphics 0.25` already declares `CGDisplayReconfigurationCallBack` (`src/display.rs:98`), so the type is there even if the register function needs checking. Pure Core Graphics, no AppKit, which would be the cheaper dependency.

`NSApplicationDidChangeScreenParametersNotification`, in AppKit (`objc2-app-kit`, `src/generated/NSApplication.rs:1883`). Same observer machinery as the foreground watcher, so the code would look like `freddie_app_nav`. But it is an `NSApplication` notification, and mercury has no `NSApplication`. Whether it is posted at all without one is unknown, and it is the same question `menu-bar.md` has to answer for `NSStatusItem`.

## Things that will bite

Reconfiguration fires more than once per change. The Core Graphics callback gets a begin-configuration pass and then a completion pass, with flags saying what happened (added, removed, shape changed). Connect and disconnect have to be read off those flags, and the begin pass has to be ignored, or every plug produces two events.

Display identity is the bundle-id problem again. `CGDirectDisplayID` is not stable across reconnects or reboots. `CGDisplayCreateUUIDFromDisplayID` gives something durable. If mercury ever wants "when *this* monitor connects", it wants the UUID, and the display-id-to-monitor table belongs with the bindings, the way the bundle-id table does.

## Open questions

- Which API, and does the AppKit one need an `NSApplication`?
- What thread does the Core Graphics callback arrive on, and does it need the main run loop?
- What does mercury do with the event? Nobody has said. A layer that only exists when docked, an app to foreground on connect, and a different keyboard set when the laptop lid is closed are all plausible and imply different state.
- Does the model want connect and disconnect as separate events, or one event carrying the current set of displays?
