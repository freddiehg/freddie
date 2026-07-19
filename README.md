# freddie

freddie is a framework for turning events into typed mutations of an application's state. laserbeam is a library within it (the typed mutable path). mercury is its first concrete use, a keyboard-remapping application that demonstrates freddie.

## Running mercury

```
cargo run -p mercury
```

macOS only, and it needs Accessibility and Input Monitoring, which the system prompts for the first time it grabs the keyboard. It runs as a menu-bar app: the icon shows the active layer, and its Quit item is the way out that does not depend on the grabbed keyboard still working.

One mercury runs at a time. A second one is refused at startup rather than allowed to fight the first over every keystroke.

Its arguments, both of which also read from an environment variable:

```
--log-level <FILTER>   what the terminal shows, default info (LOG_LEVEL)
--port <PORT>          the loopback event socket, default 3883 (MERCURY_PORT)
```

## The layers

Every key goes through the model, so what a key does depends on the layer it is pressed in. `escape` returns to home from anywhere, `o` shows the current layer's keymap, and any layer left idle returns home on its own.

- Home: `n` nav, `t` typing, `i` in-app, `u` site, `r` resize, `q` quit.
- Nav: `c`, `f`, `g`, `z` bring up Chrome, Finder, Ghostty, Zed, and land in that app's in-app layer once it is actually frontmost.
- Resize: the arrows place the focused window, then it returns home.
- Typing: every key passes through, so this is where you type.
- In-app: keyed on the frontmost app. Chrome binds `r` to refresh; Ghostty binds `j` and `k` to tmux's windows and `1` through `0` to the first ten.
- Site: keyed on what the frontmost app has open, which for Chrome is the front tab's URL. On `claude.ai`, `n` starts a new chat.

The last two hold no app and no site of their own. Both rebuild from the root on every dispatch, so there is one copy of what is frontmost and nothing to keep in sync.

## The event socket

mercury listens on `127.0.0.1:3883` so processes outside it can push events in. Anything that can hold a WebSocket can drive it:

```
{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}
```

`IncomingEvent` is the whole external vocabulary, so a sender can report the front tab and nothing else; `MercuryEvent` does not derive `Deserialize`, which is what makes remote key injection and remote quit unrepresentable rather than filtered. Web pages are refused at the handshake, because a WebSocket handshake is exempt from the same-origin policy and any open tab could otherwise reach the socket.

`chrome-extension/` is the first client, pushing the front tab's URL. Because the socket is the interface, per-site binds can be developed and tested without it.

## Logs

mercury writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and always down to `debug` whatever the terminal was asked for. One record per dispatched event carries the event, the effects it produced, and the resulting state, so a run is reconstructable afterwards.

```
tail -f ~/Library/Logs/mercury/mercury.log
```

## The crates

- `laserbeam`: the typed mutable path.
- `bind`, `bind_macro`, `derive_support`: bindings from a trigger to a handler, and the derives that build them.
- `freddie`: the framework itself, over the two above.
- `freddie_keys`: keys, presses, and modifier flags.
- `freddie_keyboard`: grabbing the keyboard and emitting keys.
- `freddie_app_nav`: foregrounding an app, and watching which one is frontmost.
- `freddie_windows`: placing the focused window.
- `freddie_menu_bar`, `freddie_overlay`, `freddie_main_loop`: the menu-bar item, the keymap overlay, and the run loop they need.
- `freddie_event_socket`: the loopback WebSocket external events arrive on.
- `freddie_single_instance`: the lock that keeps one mercury running.
- `mercury`: the application.

## Where code goes

mercury is one consumer of freddie, not freddie itself. figaro is another, and there will be more. So the test for whether something belongs in mercury is whether figaro would write it differently: if figaro's copy would be identical, it does not belong in mercury, it belongs in a `freddie_*` crate that both depend on.

What mercury keeps is what is only true of mercury: its `App` enum, its state tree, its bindings, its effects, and the table mapping bundle ids onto its apps. What it does not keep is anything about how macOS works. Grabbing the keyboard, foregrounding an app, watching the frontmost app, and giving the main thread to a run loop are all identical in figaro, and each lives in its own crate.

This is easy to get wrong, because the first consumer is the only consumer and everything looks app-specific while it is the only thing there. The rule is about the second consumer, before it exists.

## Planning

Work is planned in `refactors/pending/` and moved to `refactors/past/` once it has shipped. A doc there is meant to carry enough detail that someone with no context can implement it without making design decisions along the way, which is why they hold before-and-after code rather than descriptions of it.

## Prior art

freddie's event loop follows two existing systems. isograph's language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. barnum goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. freddie's difference from barnum is that its handlers mutate state directly during dispatch, where barnum's only return a value the engine writes back. See `refactors/pending/event-loop.md` for detail.
