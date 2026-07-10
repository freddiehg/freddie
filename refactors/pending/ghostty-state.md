# knowing what Ghostty and tmux are showing

Getting events for which Ghostty window and tab are focused, and which tmux session, window, and pane is inside them.

A note. Some of it is measured, and where it is not, it says so.

## Why

mercury's Ghostty bindings assume tmux. `i j` sends `ctrl-a p`, and `i 1` sends `ctrl-a !`. In a Ghostty tab where tmux is not running, `ctrl-a` is zsh's beginning-of-line and the command key is typed as a character, so `i j` moves the cursor and types `p`. The bindings are wrong there, silently, and the model has no way to know.

More generally, a binding should be able to depend on what the terminal is showing, not merely on which app is frontmost. Which is the same shape as the in-app layer already: `App::Ghostty` picks `GhosttyApp`, and something inside `GhosttyApp` should pick the tmux layer or the bare-shell layer.

## Two layers of state, and only one of them is visible

The operating system can see Ghostty's windows and tabs. It cannot see tmux at all. tmux's sessions, windows, and panes are text inside one terminal, and nothing outside the tmux server knows they exist.

So there are two mechanisms, and no single one answers the question.

## What the OS can tell us

The Accessibility API, and it is better shaped than `NSWorkspace` was. `accessibility-sys` exposes `AXObserverCreate`, `AXObserverAddNotification`, and, crucially, `AXObserverGetRunLoopSource`. That last one hands us the source, so we add it to whichever run loop we like, the way `CGEventTap` does and `NSWorkspace` refuses to. An AX observer does not need the main thread.

The notifications we would want exist: `kAXFocusedWindowChangedNotification`, `kAXFocusedUIElementChangedNotification`, `kAXWindowCreatedNotification`, `kAXUIElementDestroyedNotification`, and `kAXTitleChangedNotification`. An observer is created per pid, so mercury would create one for Ghostty when Ghostty appears and drop it when Ghostty goes away. That is an owned source in the sense of timer-events.md, and `Drop` would deregister it.

Whether Ghostty's tabs appear as an `AXTabGroup`, and whether switching tabs raises `kAXFocusedUIElementChanged`, is unmeasured.

## What the OS cannot, and the title as a lossy channel

Ghostty's window title right now is `tml`, which is a tmux session name. So the title does carry some tmux state, and `kAXTitleChangedNotification` would deliver it for free with no cooperation from tmux.

It is not enough. `set-titles` is off in this config, so what lands in the title is whatever the shell or tmux happens to emit, and it is a session name rather than a session, window, and pane. Building on it means parsing a string that a prompt change can silently alter. It is worth knowing the channel exists, and worth not depending on it.

## What tmux can tell us

tmux knows everything and will say so, three different ways.

Polling. `tmux list-panes -a -F '#{session_name} #{window_index} #{pane_index} #{pane_id} #{pane_active} #{pane_current_command}'` prints the whole tree, and `display-message -p` prints one field. We just deleted a poll from `freddie_app_nav` and should not add one back.

Hooks. tmux has them, `after-select-pane` and `after-select-window` among many, and a hook runs a shell command. `set-hook -g after-select-window 'run-shell "mercury notify window #{window_id}"'` pushes the change to us the moment it happens. This is the push direction effects-and-events.md argues mercury needs, and it needs an inbound source: a socket, or a CLI subcommand another process invokes. That inbound source is the enabling piece for tmux, for a Chrome extension, and for anything else that will not be observed from outside.

Control mode. `tmux -C` is a client that emits `%`-prefixed notifications, among them window and pane changes, as they happen. It needs no configuration in the user's tmux. The catch is that a control-mode client is a real client and participates in size negotiation, so attaching one can resize the session to the smallest attached client. Whether that bites in practice is unmeasured, and it is the thing to check before choosing this over hooks.

## Identity, and for once it is easy

tmux ids are stable and durable within a server: sessions are `$1`, windows are `@5`, panes are `%3`. They survive renaming and reordering, which is exactly what `CGWindowID`, `AXUIElement`, and `AudioDeviceID` do not.

The OS side keeps its usual problem. An `AXUIElement` for a window is not stable across restarts, and Ghostty's tabs have no identity the OS will hand out.

## What this does to the model

`GhosttyApp` stops being a unit struct. It holds what the terminal is showing, and its bindings depend on that:

```rust
struct GhosttyApp {
    #[resolve_into]
    contents: TerminalContents, // Tmux(TmuxLayer) | Shell(ShellLayer)
}
```

Then `j` is bound on the tmux layer and nowhere else, and a bare shell tab binds nothing, so `i j` in a non-tmux tab does nothing rather than something wrong. Which is the whole point.

If mercury ever wants to hold the set of panes rather than just the active one, that is a `Vec<Pane>` selected by an active id, which is exactly laserbeam-state-controlled-children.md.

`GhosttyApp` would also own its sources, both the AX observer and whatever listens to tmux, so entering the layer registers them and leaving drops them. See timer-events.md on owned versus shared sources. Whether that is right or whether these should be process-lifetime sources feeding events that the model merely records, like `Foregrounded` does today, is undecided.

## Open questions

- Hooks or control mode? Hooks need the inbound source and edit the user's tmux config. Control mode needs neither but attaches a client, which may resize the session.
- What is the inbound source: a unix socket, or a `mercury notify` subcommand that writes to one?
- Do Ghostty's tabs show up in Accessibility at all, and does switching them raise a notification?
- Does mercury need the whole tree, or only "is tmux in front, and which pane"? The bindings only need the latter, and the whole tree is what an overlay would want.
- Does `GhosttyApp` own its sources, or do they run for the process lifetime and merely feed events?
- What happens when a tmux command is sent to a pane that has since gone away, or to a window that is no longer there? The effect is fire-and-forget and nothing reports failure.
