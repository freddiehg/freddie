# freddie

freddie is a set of tools for building a program, specific to your computer, that ingests a stream of events and produces a stream of effects.

Events are things like: this key was pressed, this app was foregrounded, this browser tab became frontmost. Effects are things like: emit this key, foreground this app, place this window, run this arbitrary code. A program built on freddie is the one place that state lives and the one place those decisions get made, instead of half a dozen small utilities that each know nothing about the others.

The obvious use is keyboard remapping, and that undersells it.

## mercury is the demo, not the product

This repository ships one such program, `mercury`, and you should not expect it to fit your use case. It navigates to Ghostty and Zed, which is useless if you do not use Ghostty and Zed. It is here to be read, run, and forked.

The reason to write a program rather than a configuration file is that configuration limits what state you can react to. Karabiner can bind keys by frontmost app; no version of its config file binds keys by frontmost browser tab. mercury does, because a browser tab is just another event arriving at the same dispatcher.

The same goes for behavior rather than bindings. `jk` returns you to the home layer, and the thing tracking that sequence is a data structure in this repository: `JK_TIMEOUT` is 200ms at `crates/mercury/src/state/mod.rs:59`. A different window, a different escape sequence, a different tie-break are all edits, not feature requests against a schema.

## Running mercury

```
cargo run -p mercury
```

That builds mercury and starts it as a detached daemon, then exits. Installed on your path, the verbs are:

```
mercury           start one, or report the one already running
mercury restart   replace the running one, which is what a rebuild wants
mercury stop      ask the running one to quit
mercury status    the running one, and its pid
mercury logs      follow the log, starting nothing
```

`stop` and `restart` take `--force`, which destroys the daemon with SIGKILL rather than asking it to quit. That runs no destructors, so a modifier a command layer swallowed is left down in whatever app was in front; it is for a daemon that no longer answers.

macOS only, and it needs Accessibility, which the system prompts for the first time it grabs the keyboard. Nothing else: the tap is an active one, `CGEventTapOptions::Default` at `CGEventTapLocation::Session`, and Input Monitoring gates listen-only taps, which observe without being able to remap anything.

One mercury runs at a time. A second is refused at startup rather than allowed to fight the first over every keystroke.

`bacon restart` rebuilds and replaces the running daemon together, so an edited binding goes live without touching a window. That is the edit loop.

## The tour

mercury comes up as a menu-bar item. The icon shows the layer you are in, and Quit lives in that menu deliberately: the way out must not depend on the grabbed keyboard still working.

It boots into typing, which binds nothing. Every key falls through to the root, runs through the `jk` sequence, and passes through, so the keyboard is normal until you ask for something.

Type `jk` quickly, inside 200ms, and you are in home. Home is a command layer: it swallows keys it does not bind, which is why it cannot be what you boot into. From there:

- `n` then `c` foregrounds Chrome. `f`, `g`, `z` do the same for Finder, Ghostty, and Zed, and each lands you in that app's own layer once it is actually frontmost.
- `r` then an arrow places the focused window: up maximizes, left and right take half the screen. Then it returns home on its own.
- `o` shows the current layer's keymap as an overlay, so you do not have to remember any of this.
- `t` returns to typing, `escape` returns to home from every layer that binds it, and `q` quits.

Once Chrome is frontmost, `r` refreshes the tab. Once Ghostty is, `j` and `k` walk tmux's windows and `1` through `0` jump to the first ten. Those bindings exist only while that app is up. Go one level finer and the frontmost URL matters too: on `claude.ai`, `n` starts a new chat.

## The model

Every event is dispatched. Dispatch mutates state and emits a set of effects, and touches nothing else.

State is a large nested enum: which layer you are in, which app is frontmost, which site that app has open. Handlers attach to branches of that tree, and a handler runs only when the state is in the shape it was attached to. In the nav layer `c` foregrounds Chrome; elsewhere in the tree `c` is not bound to that at all.

Two things follow from that shape.

The model is a pure function of state and event, so the whole keymap is checkable as a table, and the tests read as documentation of the keymap.

And a handler receives a typed path from its own leaf up to the root:

```rust
pub(crate) fn open_chrome<'a, E, P: Ascend<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    navigate(node.parent, App::Chrome)
}
```

Everything the branch already matched on is available, typed. States that cannot happen are not unwrapped and panicked on; they are not reachable from where the handler sits.

## The edges

Inside is pure. Outside is arbitrary code, and that is the point.

Sources are whatever can produce an event: the keyboard grab, the frontmost-app watcher, and the menu-bar item are the ones built on macOS APIs. Watching a file and emitting an event when it changes would be another. Effects are the same in reverse.

mercury also listens on `127.0.0.1:3883`, so processes outside it can push events in. Anything that can hold a WebSocket can drive it:

```
{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}
```

`chrome-extension/` is the first client, pushing the front tab's URL, which is how the site layer knows where you are.

`IncomingEvent` is the whole external vocabulary, so a sender can report the front tab and nothing else. `MercuryEvent` does not derive `Deserialize`, which is what makes remote key injection and remote quit unrepresentable rather than filtered. Web pages are refused at the handshake, because a WebSocket handshake is exempt from the same-origin policy and any open tab could otherwise reach the socket.

## Starting at login

```
cargo install --path crates/mercury
mercury install
```

`install` registers the binary that ran it as a per-user LaunchAgent, so mercury starts with the session and comes back if it crashes. It copies no binary anywhere, which is why `cargo install` comes first. `mercury uninstall` takes the agent back out.

The agent needs its own Accessibility grant the first time it runs, because it has no terminal in its ancestry to inherit one from. That grant is keyed to the installed path, so a later `cargo install --path` over the same path keeps it.

Booting into typing is what makes this safe: a login that came up in a command layer would swallow every key you pressed.

Under the agent, a rebuild wants `launchctl kickstart -k gui/$(id -u)/hg.freddie.mercury`, which replaces the process launchd is managing. `mercury restart` would spawn a replacement launchd did not start and will not keep alive.

## Logs

mercury writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and always down to `debug` whatever the terminal was asked for. One record per dispatched event carries the event, the effects it produced, and the resulting state, so a run is reconstructable afterwards.

```
mercury logs                 records at info and above
mercury logs --level debug   widen that
```

Every record carries the pid of the process that wrote it, because a client verb and the daemon both append to the one file.

## The crates

- `bind`, `bind_macro`, `derive_support`: bindings from a trigger to a handler, and the derives that build them.
- `laserbeam`: the typed mutable path the bindings are built over.
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

freddie's event loop follows two existing systems. isograph's language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. barnum goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. freddie's difference from barnum is that its handlers mutate state directly during dispatch, where barnum's only return a value the engine writes back. See `refactors/past/event-loop.md` for detail.
