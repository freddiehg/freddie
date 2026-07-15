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

-----

_The user wants a broad catalog of what people actually do with Karabiner and similar keyboard/input remapping tools. Let me think about this properly and organize by category. This is a factual/knowledge question, no need to search — I know this domain well.

Let me be comprehensive and cover the real breadth. Let me organize into categories.A catalog, organized by mechanism/category. I've tried to cover the actual breadth including the weird tail, not just the greatest hits. Some of these overlap tools (Karabiner, QMK/ZMK for firmware, Hammerspoon, BetterTouchTool, AutoHotkey, kanata, kmonad, espanso).

## Simple key remaps (1:1)
1. Caps Lock → Escape
2. Caps Lock → Control
3. Caps Lock → Escape (tap) / Control (hold) — the dual-role classic
4. Caps Lock → Hyper (⌘⌥⌃⇧ all at once)
5. Right Command → something useful (it's wasted otherwise)
6. Swap Command and Option (for ex-Windows/Linux users)
7. Swap Control and Command (partial Emacs-in-everything)
8. Left Control → Escape or Control dual-role
9. Enter/Return → Control (hold) / Return (tap)
10. Both Shifts → Caps Lock (since Caps got stolen)
11. Fn key relocation / making Fn a real modifier
12. Backslash or Tab → modifier
13. Space → Shift (hold) / Space (tap) — "space cadet"
14. Swap Escape and backtick
15. Disable a key entirely (e.g. kill Caps, kill the Insert key)
16. Right Option → compose key / dedicated symbol modifier
17. Menu key → something (Windows keyboards)
18. Remap the Globe/Fn behavior on Apple keyboards

## Layers (the core power feature)
19. Home-row nav layer: hjkl or wasd → arrows under a hold key
20. Symbol layer: easy access to `{}[]()<>` etc. without reaching
21. Number/numpad layer over the right hand
22. Function-key layer (F1–F12 without a function row)
23. Media layer (play/pause/vol/brightness)
24. Mouse-move layer (move cursor with keys)
25. Navigation layer (Home/End/PgUp/PgDn/word-jump)
26. Window-management layer
27. Emoji/Unicode/symbol-insertion layer
28. Momentary vs. toggle/locked layers
29. One-shot layers (fire once then auto-exit)
30. Nested layers (layer-tap into another layer)
31. "Leader key" sequences (vim-style: tap leader, then a sequence)
32. App-launch layer (one hold + letter = launch/focus app)

## Home-row mods / advanced dual-role
33. Home-row mods: a/s/d/f and j/k/l/; become Ctrl/Alt/Cmd/Shift on hold
34. Tap-hold on nearly any key
35. Modifier-tap: hold for mod, tap for the letter
36. Tuning tap-hold timing (term, permissive-hold, hold-on-other-press) to kill misfires
37. Combos/chords: press two adjacent keys together → a third action
38. Adaptive keys / sequential remaps (key B behaves differently right after key A)

## Modifier-conditional bindings
39. Simultaneous key press (e.g. `f`+`j` together → Escape)
40. Long-press vs short-press distinctions
41. Double-tap Shift → Caps Lock toggle
42. Triple-tap or N-tap actions
43. Hold-duration-dependent output (short/medium/long → different keys)

## Per-application / context-conditional
44. Different bindings per foreground app (the thing you mentioned)
45. Terminal-only bindings (e.g. make Cmd+K clear behave right)
46. Browser-only bindings (Cmd+L, tab nav, back/forward)
47. IDE/editor-specific remaps (VSCode, JetBrains, Vim)
48. Disable Caps→Ctrl remap *inside* a specific app that needs raw Caps
49. Per-device rules (external mechanical keyboard vs. laptop keyboard get different maps)
50. Condition on keyboard type (ANSI vs. ISO, Apple internal vs. third-party)
51. Condition on connected/paired state (only when docked)
52. Condition on input source / keyboard language (US vs. non-US layout)
53. Condition on a variable/mode flag you set yourself
54. Window-title-based conditions (not just app, but which window)
55. Gaming profile that disables all your fancy remaps so games see raw keys

## Emacs / readline emulation everywhere
56. Ctrl+A/E (line start/end) system-wide
57. Ctrl+P/N/B/F as arrows
58. Ctrl+H as backspace, Ctrl+W delete-word, Ctrl+U kill-line
59. Ctrl+D forward-delete
60. Ctrl+G as Escape
61. Full readline nav layer bolted onto every text field

## Vim-style anywhere
62. Global vim navigation mode (hjkl scroll/move outside of Vim)
63. A "normal mode" toggle for system-wide modal editing
64. Escape mapped conveniently for heavy Vim users (jk, Caps, etc.)

## Text expansion / macros / typing
65. Static text expansion (`;email` → your address) — espanso territory
66. Dynamic snippets (date/time insertion, clipboard, shell output)
67. Auto-correct personal typos globally
68. Type Unicode/emoji by name
69. Accented-character / diacritic input via compose
70. Insert boilerplate (signatures, code snippets, licenses)
71. Macro: one key → a sequence of keystrokes
72. Macro: one key → keystrokes + delays (for dumb apps/games)
73. Password/2FA-adjacent quick-fill (people do it; security-questionable)

## Window & workspace management
74. Move/resize windows to halves/quarters/thirds via hotkey
75. Move window to next display
76. Switch spaces/desktops
77. Focus-follows-key app switching (hotkey → specific app)
78. Tiling-WM-style keyboard control (yabai + skhd combo)
79. Cycle windows of the same app

## Mouse & pointer from the keyboard
80. Keyboard-driven mouse movement (with acceleration curves)
81. Keyboard click / right-click / middle-click
82. Scroll via keys
83. Remap actual mouse buttons (side buttons → back/forward, modifiers)
84. Mouse-button + key combos
85. Scroll-wheel behavior changes (natural/reverse per device, or → zoom)

## Chording / stenography-adjacent
86. Custom chord dictionaries (press key-groups → whole words)
87. Steno-lite input for common phrases

## Media, hardware, system control
88. Volume/brightness/media on a layer
89. Launch scripts/shell commands from a key (Hammerspoon, skhd)
90. Trigger Shortcuts.app / AppleScript / Automator
91. Toggle system settings (dark mode, do-not-disturb, wifi) from a key
92. Control external hardware / smart home via a key → script
93. Sleep display, lock screen, start screensaver hotkeys

## Feedback / visibility / debugging
94. Display current layer/mode in the menu bar or an overlay (your "url bar" idea — people surface state in Hammerspoon menubar, Übersicht, KeyCastr-style HUDs)
95. On-screen keystroke display (KeyCastr) for screencasts/demos
96. Visual layer cheat-sheet overlay that pops on hold
97. Sound/haptic feedback on layer switch
98. Event viewer / key-code inspector to build new rules

## Ergonomics / accessibility / niche
99. Sticky keys done right (one-shot modifiers) to avoid chording strain
100. Auto-Shift (long-press any letter → capital), key-repeat rate tuning, debounce for failing switches, and "achordion"-style same-hand-mod suppression

A few structuring notes for your demo, since coverage is the goal:

The genuinely load-bearing primitives are tap-hold (dual-role), layers, combos/chords, and per-app conditions. Almost everything above is a composition of those four plus an action type (send key / send sequence / run script / set variable). If your demo makes those four first-class and composable, you cover ~80% of real configs by surface area.

The thing most remapping demos botch is timing semantics. Tap-hold and combos live or die on the resolution rules (permissive hold, hold-on-other-key-press, tapping term, chord timeout). A demo that shows a binding working but hand-waves *how it decides tap vs hold* will read as a toy to anyone who's actually fought Karabiner or QMK. Worth making that visible/tunable even in a demo.

One category you didn't list that's worth including for breadth: **stateful/variable-driven** behavior (set a flag with one key, other keys read it). It's what separates a remapper from a mini programmable input layer, and it's how people build modal systems, leader keys, and "gaming mode" toggles. Also **feedback/visibility** (your url-bar point generalized) — showing internal state is underrepresented in most tools and is a natural differentiator.
