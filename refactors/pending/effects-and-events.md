# effects and events we want

A grab bag of the sources and sinks mercury should eventually have. Not a plan and not a design. Nothing here is measured, and several entries turn out to be much harder than they sound, which is the main reason to write them down together.

This supersedes event-loop.md, whose model of dispatch mercury did not build. See the last section for what that doc still carries.

## Events

Which tab is focused in a terminal. Ghostty's tabs are AppKit tabs, but tmux's panes are not: tmux is a process, and the OS knows nothing about which pane is active. Nothing can observe it from outside. tmux has to tell us, through control mode (`tmux -C`, which emits `%window-pane-changed` and friends) or a hook that pokes mercury.

Which tab is foregrounded in Chrome. Chrome exposes no observation API. AppleScript can ask (`tell application "Google Chrome" to get active tab of front window`) but asking is a poll, and we just deleted a poll. Being told requires an extension with a native messaging host. This is the same shape as tmux: the app has to push.

An external monitor connected or disconnected. Its own note, display-events.md.

## Effects

Resize and rearrange windows. The Accessibility API, `AXUIElementSetAttributeValue` on `kAXPositionAttribute` and `kAXSizeAttribute`, against windows of `AXUIElementCreateApplication(pid)`. We already hold the Accessibility permission for the tap. This is the most tractable thing on the list.

Send keys or a message to a specific tab. Note that our `Emit` cannot do this. `CGEventPost` puts a key into the system's input stream, where it goes to whatever is focused, so "send to a specific tab" is not a variant of emitting a key. Either focus the target first and then emit, which is racy, or speak to the app directly: `tmux send-keys -t <target>` for tmux, AppleScript or the extension for Chrome. The general form of this effect is "message an app", not "emit a key".

Focus a specific terminal, tab, or window. Foregrounding an app is done. Focusing a window within it is `kAXRaiseAction` through Accessibility. Focusing a tab within a window is per-app again: tmux `select-pane`, Chrome via AppleScript or extension.

An overlay, a screen border, and showing or hiding them. All the same object: a borderless, transparent, click-through `NSWindow` (`ignoresMouseEvents`, a high `windowLevel`, `canJoinAllSpaces`). A border drawn around the screen is the obvious way to show which mode we are in, which is the thing modal editors get right and modal keyboards usually do not. This is AppKit, so it wants the main thread, and it probably wants an `NSApplication`, which is the unmeasured question `menu-bar.md` also has.

Modifier states. `modifier-keys.md` already scopes this: cmd is not special, it is a key whose down and up are transitions, and `WithModifierKeys<T>` tracks what is held. Listed here only so the grab bag is complete.

Keyboard-mouse mode. Move and click the pointer from the keyboard. `CGWarpMouseCursorPosition` to move, `CGEvent` mouse events to click. It is a state in the model, and it needs continuous motion, which means a timer source feeding repeat events rather than one event per keypress.

## What is actually hard here

Three things recur, and they are worth seeing before any of these is built.

The first is identity. Bundle ids solved this for apps. Windows have `CGWindowID` and `AXUIElement` references, neither stable across restarts. Tabs have no OS-level identity at all. Any effect that targets "that tab" needs an identity scheme the target app agrees to, which usually means the target app has to be complicit.

The second is direction. Foregrounding and the keyboard are things we observe from outside. Terminal panes and browser tabs cannot be observed from outside; the app must push to us. That means mercury needs an inbound source that is not an OS observer: a socket, or a CLI subcommand that another process invokes. That is a genuinely new kind of source, and it would serve tmux, a Chrome extension, and anything else we ever want to hear from. It is probably the single most enabling item on this page.

The third is that most of these effects are slow and involve other processes. AppleScript, `tmux`, `open`. They must stay fire-and-forget off the effect loop, the way `Foreground` already spawns a thread, and their results must come back as events rather than return values. Also, several want Automation permission, which is a second TCC prompt, and Chrome's `execute javascript` additionally requires "Allow JavaScript from Apple Events" to be turned on by hand.

## What event-loop.md still carries

event-loop.md is superseded as a description of mercury, because mercury does the opposite of what it prescribes. It says dispatch happens synchronously inside the tap callback so key output is the callback's return value and is never re-posted. mercury sends the event down a channel, returns `None`, dispatches on the worker thread, and re-posts the output through `CGEventPost`.

The consequence is real and should not be lost with the doc. Because mercury re-posts rather than returning the event down the tap chain, it has the cross-process loop hole `cgevent-vs-hid.md` describes. The `EVENT_SOURCE_USER_DATA` tag stops mercury re-eating its own output, but it does not stop another process feeding that output back. Two remappers with inverse maps would ping-pong. We accept this because we are the only remapper on the machine, and nobody has written that down until now.

Whether to move to the synchronous model, and whether it is even compatible with dispatching on a single worker thread that owns the state, is the open question that outlives the doc.
