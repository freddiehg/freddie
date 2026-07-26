# sending things to an agent

Agents run in tmux panes: session `tml` (and `tmr`), windows `gg1`–`gg3` and `t5`–`t15`, one pane per window, per `~/.tmuxp/tml.yaml`. mercury holds an active agent in state, and "send this to the agent" bindings deliver to that agent's pane, whichever app is frontmost, because `tmux send-keys -t <target>` needs no focus at all. The things worth sending: text off the clipboard, the last screenshot's path, the last download's path, and live dictation through Wispr Flow.

## The state

```rust
/// A tmux window an agent lives in. The set is fixed by the tmuxp config, so the type is an
/// address, not a discovery: session and window name, straight into `-t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentPane {
    pub session: TmuxSession,
    pub window: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmuxSession {
    Tml,
    Tmr,
}

impl AgentPane {
    /// `tml:gg1`, the string `-t` takes.
    pub fn target(&self) -> String;
}
```

On the root: `active_agent: Option<AgentPane>`. `None` means no agent has been chosen since boot, and the send bindings do not bind (the same virtual-field trick as `app_data`: no agent, no send layer). Choosing an agent assigns it; there is no history and no toggle until one is wanted.

Choosing happens in an agent layer: enter it from home, pick a window (`1`–`3` for `gg1`–`gg3`, then a scheme for `t5`–`t15`; the keys are the user's call), and leave for home, because picking is one decision. The overlay lists the panes while choosing.

## Delivery

Delivery is `MercuryEffect::Run` (run-effect.md) with tmux as the program; no new effect variant until clipboard reading forces one (below).

- A path (screenshot, download): `tmux send-keys -t tml:gg1 -l <path>`. Literal, no key-name interpretation. Claudes take a pasted path and read the file, so sending the path is sending the artifact.
- Text: same, `-l <text>`.
- Submitting: a second `send-keys -t <target> Enter`. Whether a send submits or leaves the message staged for the user to finish is a per-binding decision, and it is an enum, not a flag:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submission {
    /// Follow the payload with Enter: the agent starts on it now.
    Submit,
    /// Leave it staged in the pane's input, to be finished or submitted by hand.
    Draft,
}
```

The default is `Draft`: a path or a paragraph usually wants words around it, and a stray Enter into the wrong Claude is the failure mode worth defaulting away from.

The clipboard payload is the wrinkle: its contents are the outside world's, and `Run`'s payload is fixed at dispatch time, so a "send clipboard" binding cannot put the text in the payload. Two candidate shapes, decision open: a performer that reads `NSPasteboard` and then runs `tmux send-keys` (an outside-world read on the effect side, same standing as the copy fallback), or `tmux load-buffer` fed from `pbpaste` piped in a shell, which is the same read wearing tmux's clothes. Either way the read happens at perform time; the choice is which process does it.

## What feeds it

- screenshots.md leaves `last_screenshot: Option<PathBuf>` in state.
- chrome-control.md's `downloads` permission reports each completed download's absolute filename as an event; the handler assigns `last_download: Option<PathBuf>`.
- Whether those stay two fields or fold into one `last_artifact` the send binding reads is decided here: two fields, two send bindings. "Send the screenshot" and "send the download" are different intents, and folding them makes the binding's meaning depend on which event fired last, which is exactly the ambiguity a keymap should not have.

## Wispr Flow

Dictating into an agent is the one payload that cannot be delivered focus-free: Wispr types wherever the cursor is. So the dictation binding is a gesture, not a message:

1. `Foreground(App::Ghostty)`.
2. `Run`: `tmux select-window -t <active agent's target>` (and the right client/session; `tml` and `tmr` are different Ghostty windows, which is a window-focus problem `freddie_windows` can take on once it matters).
3. `Tap` Wispr's trigger key, the one its hotkey matcher listens for.
4. End in typing, because what follows is speech landing as text, and a command layer would swallow any manual cleanup after it.

The foreground step is asynchronous (the watcher confirms it), so step 3 must not fire before Chrome-style confirmation: the gesture parks in a waiting state and the `Foregrounded(Ghostty)` event releases the tap, the same shape as the rewrite state machine in effects-and-events.md. Racing it and taping into the old app is the bug this exists to avoid.

This same gesture minus the Wispr tap is plain "go to the agent": foreground Ghostty, select its window, end in typing. That binding is wanted anyway and is the cheap one to build first.

## Open questions

- The key scheme for choosing among thirteen panes per session, and whether `tmr`'s panes appear at all or the active session is itself state.
- Which process reads the clipboard at send time (performer via `NSPasteboard`, or `pbpaste | tmux load-buffer -`).
- Whether an agent picker should show liveness (which windows have a Claude running vs a bare shell), which would need the tmux state channel ghostty-state.md describes and is explicitly not required for v1.
