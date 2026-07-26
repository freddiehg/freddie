# controlling chrome

The channels mercury has into Chrome, what each one can and cannot carry, and which one carries what. This is the survey behind the plan that already exists: external-effects.md gives the socket a write direction, scoped-commands.md scopes the command vocabulary to the state tree, and extension-commands.md is the browser half. This note does not repeat those; it answers the question they assume: why the extension is the channel, and what stays outside it.

## The channels

There are six ways to make Chrome do something, and mercury already uses three of them.

### Keystrokes

`MercuryEffect::Tap` through the emitter. This is what the Chrome layer does today: `cmd-r` to refresh, `cmd-l` for the address bar, `cmd-shift-o` on claude.ai. It reaches whatever has focus, which is its whole character: it cannot address a background tab, it cannot carry an argument, and nothing comes back. When the front tab is the target and the site already binds the action, it is the cheapest channel there is, and per ideas.md it is also the only integration point a browser extension like Dark Reader offers (`chrome://extensions/shortcuts` plus a `Tap`).

### The extension over the event socket

The MV3 extension in `chrome-extension/` already holds the socket open and pushes the front tab's URL in. The write direction is designed in external-effects.md: `OutgoingEffect::Command(BrowserCommand)` serialized down the same WebSocket, addressed by the `TabId` the extension reported. This is the only channel that solves the identity problem, because the extension lives inside Chrome and speaks Chrome's own tab ids, which are stable for the life of the tab and mean the same thing on both ends of the socket.

Latency is a frame on an already-open loopback WebSocket plus the service worker handling it: milliseconds, and no process spawn anywhere.

### AppleScript

`osascript` reaches Chrome's object model: `tab 3 of window 1`, `every tab whose URL contains "meet.google.com"`, `execute javascript` (behind the hand-toggled "Allow JavaScript from Apple Events"). It is the only channel besides the extension that can read a background tab's URL. It is also a process spawn plus an Apple Events round trip per question, tens of milliseconds when warm, occasionally seconds, and it needs the Automation TCC grant.

It stays exactly where it is: the fallback inside `copy()` when no URL was reported, documented as the one Apple Events use. Nothing new is built on it. Once the extension reports tabs reliably (audio-during-a-meeting.md's `IncomingEvent::Tabs` reports all of them), the fallback's reason to exist goes away and it can be deleted.

### CDP through the debugging port

Chrome's DevTools Protocol over `--remote-debugging-port` is off the table for the daily browser: since Chrome 136 the flag is ignored on the default user data dir, so driving the real profile over the port would mean relaunching Chrome into a separate profile, which is not the browser being controlled. A dedicated automation profile could still use it; mercury has no use for one.

### CDP through the extension

`chrome.debugger` gives the extension most of CDP against a tab it attaches to, with no launch flags and no separate profile: `Page.captureScreenshot` for a full-page screenshot, `Input.dispatchKeyEvent` aimed at a background tab, `Runtime.evaluate` in the page's own world rather than the isolated one. The cost is the "is being debugged by" infobar for the duration of the attachment, and the `debugger` permission. So CDP is not a separate channel; it is a capability the extension picks up when a command needs it, attach-act-detach.

### Accessibility, and `open`

AX sees what is drawn: windows, titles, the visible tab strip, never a background tab's URL. mercury already uses it for window frames, and that is what it is for; it is not a tab channel. `open -b com.google.Chrome` foregrounds the app and `open -n -b com.google.Chrome --args --profile-directory=...` is how link-dispatch.md reaches a specific profile; both are launch-and-focus, not control.

## What the extension can carry

The manifest today asks for `tabs`, `storage`, `webNavigation`. Each capability below names the permission it adds. Adding a permission to an unpacked extension disables it until re-approved on `chrome://extensions`, which is a one-click cost per change, so permissions should land in batches alongside the commands that use them.

Reading, with no new permissions: every tab's id, url, title, `audible`, `active`, window id (`chrome.tabs.query({})`); window focus changes; navigation. This is the `IncomingEvent::Tabs` report audio-during-a-meeting.md designs.

Acting on tabs, with no new permissions: activate (`chrome.tabs.update(id, { active: true })` plus `chrome.windows.update(windowId, { focused: true })`), create, close, reload, move, duplicate, navigate (`update` with a `url`).

Running script in a page: the `scripting` permission plus host permissions for the sites involved (today the extension holds only `http://127.0.0.1/*`). `chrome.scripting.executeScript` reads and writes the DOM in an isolated world: read the focused editor's contents, replace them, click a button by selector. This is what select-all-rewrite.md rides. Whether host permissions grow site by site (`https://claude.ai/*`) or go to `<all_urls>` once is a decision to make when the second site arrives; site by site matches how the site layer grows and keeps the grant legible.

Screenshots: `chrome.tabs.captureVisibleTab` needs `<all_urls>` (the `activeTab` alternative requires a user gesture on the extension itself, which mercury never produces). Full-page, beyond the viewport, is `chrome.debugger` + `Page.captureScreenshot`. Both return the image over the socket as base64, which the 64 KB frame cap does not fit; screenshots.md decides between raising the cap and having the OS take the picture instead.

Downloads: the `downloads` permission. `chrome.downloads.onChanged` reports a completed download's absolute filename, which becomes an `IncomingEvent` and lands in state, where send-to-agent.md picks it up.

What the extension cannot do, ever: `chrome://` pages, the Web Store, other extensions' pages (`chrome.tabs.update` can navigate to a `chrome://` URL, but no script runs there); anything while Chrome is not running, because the service worker is Chrome's process; anything in a profile the extension is not installed in. Each profile that has it runs its own service worker and opens its own socket connection, which the one-slot `Browser` reporter in external-effects.md already accommodates: the front tab's reporter is whichever connection spoke last.

The service worker also dies after ~30s idle. Since Chrome 116 all WebSocket activity resets that timer, so extension-commands.md's `chrome.alarms` keepalive at 0.5 min, plus traffic itself, keeps the socket held open. A killed worker reconnects on its next wake and pushes the front tab on `open`; the gap is real but self-healing.

## Which channel carries what

The rule that falls out:

- The front tab is the target and the site binds a shortcut for the action: `Tap`. No addressing, no reply, no permissions.
- The action is addressed at a tab (background or not), carries an argument, needs the DOM, or needs an answer: a `BrowserCommand` over the socket. This is everything else: activate that tab, mute the Meet, rewrite the prompt box, report all tabs.
- Full-page capture and background-tab input: still a `BrowserCommand`, performed by the extension via `chrome.debugger`.
- Launching Chrome, or opening a URL in a chosen profile: `open`, per link-dispatch.md.
- AppleScript: the existing copy fallback only, until `Tabs` reports make it deletable.

So the answer to "does everything go through the extension" is: everything programmatic does, because it is the only channel with real tab identity and millisecond latency, and it is one hop from state the model already keeps. Keystrokes stay for what the focused page already binds, and `open` stays for what happens before Chrome is listening.

Direction stays asymmetric on purpose. Commands flow out as effects and are fire-and-forget; what the browser knows flows in as events and lands in state. A command never has a return value: activating a tab produces a `Tab` event because the activation happened, not because the command replied. That keeps dispatch a function of `(state, event)` and keeps the extension dumb on the reply side.

## Two concrete asks, placed

Opening a tab to a named site, "a tab to YouTube": today this is `open -b com.google.Chrome https://youtube.com`, which mercury can already say (`freddie_app_nav` shells `open`; run-effect.md generalizes it), and it foregrounds Chrome as a side effect, which is what a "go to YouTube" binding wants anyway. Its weakness is profile blindness: `open` lands in the last-active profile. Once extension commands exist, the addressed form is `chrome.tabs.create({ url })` as a `BrowserCommand`, and link-dispatch.md's profile routing picks the window. The binding lives in the Chrome in-app layer (or nav, if it should work from anywhere), and it leaves for typing or stays per the where-a-binding-leaves-you rule; a fresh tab showing a site you now use is an in-app destination, so `and_go_home` into in-app is the default.

Opening DevTools: Chrome binds `cmd-opt-i`, and the extension has no API that opens the DevTools UI, so this is a `Tap` and nothing more, a Chrome-layer binding next to `cmd-r`. The same goes for the JavaScript console (`cmd-opt-j`) if it earns its own key.

## The socket once commands flow

external-events.md declined an auth token while the socket only received reports, and said the calculus changes when it carries commands. It does: a local process that connects and speaks the outgoing frame format could drive the browser. The exposure is loopback-only and the commands are a closed vocabulary (no arbitrary script travels the wire; `BrowserCommand` variants name site-scoped actions), so the risk is bounded, but a shared-secret token minted by mercury and handed to the extension through its options page is cheap and should ride along with extension-commands.md.

## What this unlocks, by doc

- extension-commands.md: the message listener, zod parsing, command routing. The prerequisite for everything below.
- audio-during-a-meeting.md: all-tabs reporting, `audible`, mute.
- screenshots.md: tab capture, and where Chrome's channel ends and the OS's begins.
- select-all-rewrite.md: site layers that read and replace the focused editor through `chrome.scripting`.
- send-to-agent.md: downloads reported into state, handed to an agent.
- switchers.md: a tab switcher built on the all-tabs report plus the activate command.
- link-dispatch.md: URLs into Chrome by profile, from outside.

## Build order

1. external-effects.md, unchanged: `Client`, `TabId`, `OutgoingEffect`, the `Browser` slot.
2. scoped-commands.md, unchanged: the command vocabulary follows the tree.
3. extension-commands.md, plus the token above.
4. The `Tabs` report (audio-during-a-meeting.md's first half), which retires the AppleScript fallback.
5. Capabilities in whatever order they are wanted: each is a new `Command` variant, a new extension routine, and possibly a permission batch.
