# effects and events we want

A grab bag of the sources and sinks mercury should eventually have. Not a plan and not a design. Nothing here is measured, and several entries turn out to be much harder than they sound, which is the main reason to write them down together.

This supersedes the effects and events half of event-loop.md, which is retired. Its model of dispatch, which mercury did not build, is synchronous-dispatch.md.

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

## More effects

Trigger Wispr Flow, and send a Wispr Flow message to a specific window. voicemode does the first by having Karabiner synthesize `F19` (from `RCtrl+y`), which Wispr listens for, plus an `F18` passthrough. Targeting a specific window is the "message a specific tab" problem again, and the same answer applies: a global key emit goes wherever focus is, so either focus first or speak to the app.

Read and write the paste buffer. voicemode's `l` layer does `pbcopy < src/llm-blurb.txt` to load a canned prompt, and `vscode-open-in-chrome.sh` does `pbpaste` to read a path back out. As effects these are trivial (`NSPasteboard`, or shelling out), and they are the cheapest way to move text into an app that will not talk to us.

Dispatch to various Claudes. This is the one that pulls everything else along. A Claude runs in a tmux pane, in a terminal window, or in a browser tab, and dispatching to a chosen one means naming it, focusing it, and delivering text to it. Naming needs the identity scheme below. Delivering needs either the paste buffer plus a synthesized paste, or `tmux send-keys -t <target>`, which is exact and needs no focus at all. tmux is the tractable case, and it is probably where this starts.

Select all, cut, hand it to an LLM to rewrite, paste the result. Every piece exists: `cmd-a` and `cmd-x` are chords the emitter already sends, the paste buffer is `NSPasteboard`, and the LLM is a subprocess or an HTTP call. Composing them is where it gets interesting, and it is the first effect that is genuinely slow rather than merely off the hot path.

It takes seconds, so it cannot be one effect. It is a state. Cutting is one effect, the rewrite finishing is an event, and pasting is the effect that event triggers. In between, the model is in a rewriting state, which is exactly the pattern `refactors/past/overall-plan.md` prescribes for anything with a delay, and exactly the shape of the `Foreground` effect whose result arrives later from the watcher.

Four things will go wrong, and each is worth designing for rather than discovering.

The text lives only in the clipboard between the cut and the paste. If the LLM call fails, times out, or mercury exits, it is gone from the document and gone from mercury. The cut has to be recoverable: keep the text in the state, restore it on failure, and treat a failed rewrite as an event that pastes the original back.

The clipboard is global and someone else's. Clobbering it to move text through is rude and detectable. Save its contents, use it, and put them back, which is a small state machine of its own.

Focus can move while the LLM thinks. Paste blindly and the rewritten paragraph lands in Slack. The state has to remember which app, and ideally which window and text field, the cut came from, and refuse to paste if the answer changed. `kAXFocusedUIElementChangedNotification` is how it would know.

And the user keeps typing. Keys arriving during the rewrite have to go somewhere: swallowed, buffered, or passed through into a document that is about to be overwritten. This is the first case where the model wants to say "I am busy" and mean it.

Once it exists, the same shape gives translation, grammar fixes, explaining a selection, and converting units, all differing only in the prompt. The prompt itself is data, which makes it a binding: `#[bind(Key::KeyT.down() => rewrite("translate to French"))]` is a value handler, which is `handlers-as-values.md`.

## What voicemode already does

The setup mercury replaces (`~/code/voicemode`) is a good inventory of the target, and a few of its details are worth stealing rather than rediscovering.

Its architecture is a file bus: Karabiner writes `/tmp/karabiner-layer` and `/tmp/karabiner-command`, and Hammerspoon watches those paths and runs handlers. That is the inbound source described below, implemented as files, and it is why `state.lua` calls itself the single source of truth and enforces valid transitions by hand. That is what laserbeam is for.

It has twenty-four layers, including an `l` family crossed with held modifiers (`lC`, `lTC`, `lTO`, `lCTO`), which is `WithModifierKeys` written out by hand, one state per combination.

It has the pointer work already: directional scroll, hover mode, six click variants, Homerow labels, a grid layer, and `warpd`. It has window management (halves, maximize, next screen) and switcher emulation. It has per-layer screen borders and overlays whose text comes from `src/layers/*.txt`, and a SwiftBar menu bar item.

Its display module is the thing to copy carefully. On external-monitor connect it disables the built-in panel through BetterDisplay, one-directionally (it never re-enables, because macOS already does and fighting it flaps), idempotently (re-firing the watcher recomputes the same action), and only after a DDC confirm, because enumerating screens proves macOS sees a display and not that the panel is lit. See display-events.md.

And it has three separate panic buttons: `cleanup-external-state.sh` resets warpd, Homerow, the scroll timer, hover mode, and the lmode modifier at once, alongside `panic-cleanup.sh` and `hammerspoon-hard-restart.sh`. A program that swallows the keyboard needs an undo-everything, and mercury has only the killswitch. See launch-at-login.md.

## Targeting a specific Chrome window or tab

Not "do something to Chrome", but "find the tab whose title contains X, in the profile window, and act on that one". Foreground it, reload it, run script in it, read its URL, close it, move it. voicemode already does the coarse version: `chrome-tab.sh` finds a window whose title contains `(Personal)` and raises it, and `focusChromeProfile` matches personal versus work by title.

There are two addressing schemes and they do different things. AppleScript sees Chrome's own object model: `window 2`, `tab 3 of window 1`, `every tab whose URL contains "github"`. That reaches inside a window to a tab and can read a tab's URL and title, which nothing else can. Accessibility sees only what is drawn: windows and their titles, the visible tab strip, but not the URL of a background tab. So finding a tab by URL is AppleScript; raising a window and clicking in it is either.

The addressing is the effect's real content, and it wants to be data. "The tab whose URL matches this" and "window N of the profile that is not this one" are queries, and an effect like `ChromeDo { target: Query, action: Action }` is the general form, with `Foreground` and the tmux commands as the degenerate case where the target is "the front one". Which is the same realization as messaging a tmux pane: the effect is "message an app at an address", and emitting a key is the special case where the address is "whatever has focus".

A concrete one: mute and unmute a Google Meet, from any app. The Meet is a tab, probably not the front one, probably in a particular profile window, and the shortcut for mute is `cmd-d` but only when that tab has focus. So the effect is: find the Meet tab by URL (`meet.google.com`), and either send it `cmd-d` or drive the mute button. Sending the key needs the tab focused, which is a foreground-then-key with the race that implies. Driving the button needs `execute javascript` to click the DOM node, which is more precise and needs the Apple Events permission. This is the whole feature in one binding: an address (the Meet tab), an action (toggle mute), and a decision about which of the two mechanisms delivers it. The same shape works for pausing whatever tab is playing audio, which Chrome exposes as an audible-tab query.

The hard parts are the ones already on this page, sharper. Identity: a tab has none the browser will hand out, so you address it by a property (title, URL, index) that the page itself changes as you use it, and the tab you found a moment ago is a different tab now. Direction: reading which tab is frontmost is the poll or the extension in the events section above, and acting on a specific one is this. Cost: every one of these is an AppleScript round-trip of tens of milliseconds, off the effect loop, and Chrome's `execute javascript` additionally needs "Allow JavaScript from Apple Events" turned on by hand.

## What is actually hard here

Three things recur, and they are worth seeing before any of these is built.

The first is identity. Bundle ids solved this for apps. Windows have `CGWindowID` and `AXUIElement` references, neither stable across restarts. Tabs have no OS-level identity at all. Any effect that targets "that tab" needs an identity scheme the target app agrees to, which usually means the target app has to be complicit.

The second is direction. Foregrounding and the keyboard are things we observe from outside. Terminal panes and browser tabs cannot be observed from outside; the app must push to us. That means mercury needs an inbound source that is not an OS observer: a socket, or a CLI subcommand that another process invokes. That is a genuinely new kind of source, and it would serve tmux, a Chrome extension, and anything else we ever want to hear from. It is probably the single most enabling item on this page.

voicemode already has this, as files under `/tmp` that Hammerspoon watches. It works, and it is the shape to keep while replacing the transport.

The third is that most of these effects are slow and involve other processes. AppleScript, `tmux`, `open`. They must stay fire-and-forget off the effect loop, the way `Foreground` already spawns a thread, and their results must come back as events rather than return values.

The obvious cleanup there is a trap. `foreground_app` uses `std::thread::spawn` where `tokio::task::spawn_blocking` looks more idiomatic and would give a bounded, reused pool for free. But dropping a tokio `Runtime` waits for in-flight blocking tasks, and a detached thread does not. Measured: three seconds versus fifty milliseconds. mercury exits by returning from `run`, dropping the runtime, dropping the `Stopper`, and stopping main's run loop, which is what releases the keyboard. On `spawn_blocking`, a hung `open` would hold the keyboard until the five second hard exit. The detached thread cannot.

So if a pool ever becomes necessary, and unbounded thread-per-effect stops being fine once AppleScript is in the mix, it has to be a pool whose workers are detached, or `spawn_blocking` with `Runtime::shutdown_timeout` at exit rather than a plain drop. Also, several want Automation permission, which is a second TCC prompt, and Chrome's `execute javascript` additionally requires "Allow JavaScript from Apple Events" to be turned on by hand.
