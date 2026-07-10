# ideas

A grab bag of sources, sinks, states, and stunts. Speculative on purpose. Nothing here is measured, most of it will never be built, and the point is to see the shape of what freddie could be rather than to plan any of it.

The things with their own docs are not repeated: window management, overlays, keyboard-mouse mode, displays, microphones, timers, tmux and Ghostty state, dispatching to Claudes.

## Sources that are nearly free

These arrive on machinery mercury already has. `freddie_app_nav` observes one `NSWorkspace` notification; the notification center has a dozen more, and each is one string constant away.

An app launched or quit (`didLaunchApplication`, `didTerminateApplication`). Chrome dying should probably drop you out of its in-app layer rather than leaving bindings pointed at a corpse.

The machine slept or woke, the screen locked or unlocked, the session was handed to another user. A keyboard remapper that survives sleep with a modifier stuck down is a keyboard remapper you reboot.

The active Space changed. Layers per Space is a coherent idea and a terrible one, in some order.

An `AXObserver` gives a run loop source, so per-app observation costs nothing structural. A window moved or resized by hand. A window was created or destroyed. The focused UI element changed.

## The focused element, which is the interesting one

`kAXFocusedUIElementChangedNotification` tells you when the cursor lands in a text field. That single fact answers the question `launch-at-login.md` is blocked on: mercury boots into `Home`, which swallows everything, so autostarting it makes the machine look broken.

A model that knows it is in a text field can pass keys through, and know it should. The layer stops being something you choose and starts being something the machine already knows. Type in Slack, get typing. Click a canvas, get nav. It is the same trick as the in-app layer following the front app, one level finer.

It is also the most likely thing on this page to be wrong, because "is this a text field" has a hundred edge cases and being wrong means swallowing someone's password.

## Sources from elsewhere in the machine

Battery, power source, and whether the lid is closed. `IOPSNotificationCreateRunLoopSource` hands you a run loop source, like `AXObserver` and unlike `NSWorkspace`.

Wifi network changed. Docked at the office versus at home is a state, and it selects a different set of apps to nav to.

A USB device appeared. A foot pedal is a keyboard with one key, and a keyboard is a foot pedal with a hundred.

Idle time. `CGEventSourceSecondsSinceLastEventType` reads how long since the last keystroke, and a state that has been sat in for ten minutes should probably not still be nav. This wants timer-events.md.

A calendar event starting. EventKit. Muting the microphone one minute before a meeting is the kind of thing you only notice when it stops happening.

The clipboard changed. `NSPasteboard` only exposes a `changeCount`, so this is a poll, and it is the one poll that might be worth it.

A file changed. An `FSEvents` watcher on a git directory tells you the branch changed, which the menu bar wants.

## Effects, small

Type a canned string. voicemode has this: an LLM prompt in a file, `pbcopy`'d by a keystroke. As an effect it is three lines and it is used constantly.

Clipboard history. Copy pushes, a layer of digits pops. The state is a `Vec<String>` and an index, which is `laserbeam-state-controlled-children.md` wearing a hat.

Paste as plain text. Everyone reimplements this. It is `NSPasteboard` minus the RTF.

Speak the current layer aloud when it changes. Sounds absurd until you use a modal editor with no visual feedback.

A sound or a haptic tick on entering a layer. The trackpad's haptic engine is reachable.

Volume, mute, brightness, dark mode, do-not-disturb. Each is one system call and each is a key you currently reach for with your thumb.

## Effects, larger

Screenshot a region, OCR it with Vision, put the text on the clipboard. The Vision framework is on the machine and this replaces a paid app.

OCR the whole screen, find every string that looks clickable, label them, and click the one you name. That is Homerow, and voicemode already shells out to it. Building it means Vision plus the overlay plus keyboard-mouse mode, and all three are already wanted.

Run a shell command and paste its output. `date`, `uuidgen`, `git rev-parse HEAD`. The dumbest possible effect and probably the most used.

Open the current file in the other editor. Zed and VS Code both take a `file:line` URL, and voicemode already has `vscode-open-in-chrome.sh`.

Remember and restore a window layout. Place is already there; the state is a map from app to frame.

Click a thing in another app's menu bar or toolbar. `osascript` can do it: `tell application "System Events" to tell process "Keeper" to click menu item ...`, and Accessibility can do it properly with `AXPress` on an element found by role and title. Triggering a password manager's fill from the keyboard, without its own hotkey, is the obvious use.

voicemode already does the Accessibility half of this. `chrome-tab.sh` finds a window by title and sends it `AXRaise` through System Events. Pressing a button is the same walk with a different action.

Two things make it harder than it reads. Finding the element means walking the app's Accessibility tree by role and title, and both are display strings, so this is the identity problem again, in the worst form: not merely unstable, but localized. And an app with no menu bar item and no Accessibility label offers nothing to click, which is where the OCR-and-label idea below stops being a stunt.

Toggle Chrome's dark mode through Dark Reader. Worth writing down because the obvious routes all fail, and the failure is instructive. AppleScript's `execute javascript` runs in the page, and an extension runs in an isolated world, so the page cannot reach it. There is no AppleScript dictionary for an extension. Clicking its toolbar button through Accessibility works and is exactly as brittle as the previous entry.

The route that works is that Chrome lets an extension declare a keyboard shortcut, at `chrome://extensions/shortcuts`. Assign Dark Reader one, and mercury's effect is a `Tap`, which it already has.

That generalizes past Dark Reader. For any browser extension, the integration point is a hotkey, not an API. Which is a little humbling: the most robust way for a keyboard remapper to talk to a program is to press a key at it.

## States

A numeric argument. `3 j` walks three panes. The state holds a count, digits accumulate into it, and the next command consumes it. Vim's best idea and nobody outside vim has it.

Leader sequences. `g` then `d` means something `g` alone does not. That is a state per prefix, which is what the model already is, plus a timer that gives up.

Sticky modifiers. Press shift, release it, and the next key is shifted. Two states and a transition, and it is what `modifier-keys.md` describes without naming.

Record and replay a macro. A state that accumulates events, and an effect that dispatches them back through the queue. Because dispatch is a pure function of state and event, replay is real replay, not an approximation.

Which-key. Enter a layer, wait 400ms, and the overlay shows what the layer binds. The delay is a timer, and the content is the accumulated trigger set. Which brings us to the good one.

## The overlay should be generated, not written

voicemode's overlay text lives in `src/layers/*.txt`, one file per layer, maintained by hand next to bindings defined somewhere else. It is wrong the moment someone adds a key and forgets.

`bind::accumulate` returns exactly the set of triggers the current state binds. Nothing consumes it. Render that set and the cheatsheet cannot drift from the bindings, because it is the bindings. Add a key, the overlay shows it. Delete one, it disappears.

That is `accumulate`'s first real consumer, and it is worth more than the registration diff it was designed for.

Further out, `laserbeam-missing-features.md` sketches enumerating every reachable state at the type level. That would generate the whole manual: every layer, every binding, every path into and out of it, from the types.

## Stunts

Log every dispatch and replay it. The log already contains the event, the effects, and the state on one line, which was for debugging. It is also a test case. Replaying a day of keystrokes against a changed model and diffing the effects is a regression suite nobody has to write.

Fuzz the model. Random event sequences, assert it never panics, never leaves a state with no way home, and never emits a modifier it did not press. The last one would have caught the flags bug.

Drive the model with no keyboard at all. `bind::SimpleRunner` already does this in tests. A REPL that dispatches typed event names would let you explore the state machine without hijacking anything.

Two keyboards, two independent layers. The laptop in typing while the external is in nav. Wants multiple `#[resolve_into]` (laserbeam) and per-device events (HID), and it is the thing both of those docs are secretly for.

Voice as a source. Wispr already produces text; a phrase could be an event, and "layer nav" is a trigger like any other.

A binding that opens the source of the binding you just pressed. The derive knows the file and line. This is either a debugging tool or an act of hubris.

## The one that is not a joke

Everything above is a source or a sink. The model in the middle does not care, and that is the whole thesis: freddie is not a keyboard remapper. `refactors/past/overall-plan.md` claims a router and a reactive UI are the same machine, and nothing has tested that claim.

The cheapest test is the smallest second consumer. Not figaro, not a browser app. Something like a state machine for a build pipeline, or a vending machine, or a traffic light, written against laserbeam and bind with no keyboard anywhere. If that is awkward, the abstraction is a keyboard remapper wearing a costume, and better to find out now.
