# `freddie`

**`freddie` is a set of tools for building a bespoke control plane for your computer**.

This program ingests a stream of events and produces a stream of effects. One such event is generated when you press a key on your keyboard, and one such effect is a simulated keypress! **So, `freddie` can be used to build a key remapper.** But the events and effects are arbitrary, and so `freddie` can be used to build something much more powerful.

Want to ensure that when you connect a specific microphone, Wispr Flow uses that one? Want to rearrange your windows when connecting to a specific monitor? Want a keybinding to mute/unmute yourself in Google meets/Zoom? Want a hotkey to send a transcribed message to a specific Claude instance? Want to be able to clone a repository, directly from github.com?

All of this is possible with `freddie`.

Example events include: this key was pressed, this app was foregrounded, this browser tab became active, this external device connected. Example effects include: emit this key, foreground this app, resize this window, run this arbitrary code. A program built on `freddie` is the central place where the decision of how to respond to an event is made.

And `freddie` aims to do this in a way that provides a great developer experience.

## mercury

This repository contains one such demo program, `mercury`, and you should not expect it to fit your use case. It is here to be read, run, studied, and forked. See below for more information.

## Alternatives

### Why `freddie`? Why not karabiner? Why not hammerspoon?

In many ways `freddie` is a replacement for Karabiner and other keyboard remappers. These are excellent programs, but they are limited in their customizability due to being configuration-driven. For example, you can bind keys differently in Karabiner based on which app is foregrounded, but not which Chrome tab is active or which devices are connected. And so, if you want to do that, you have three bad options:

- emit a (hopefully unused) keypress that changes some internal Karabiner state, and make sure to keep that state in sync!
- bind all keys for all states, and then have the handler know what to do, or
- use an external program, such as hammerspoon.

Most folks will choose the third option, leading to a spaghettification of configuration code, and a difficulty reasoning about the overall state.

### What alternative is there to being configuration-driven?

With Karabiner, you download a binary and provide a configuration. With `freddie`, you fork the repository, make the changes you want, and run `cargo build` to generate the new binary.

That gives the freedom to do whatever you want!

This comes at a cost, and `freddie` is not for everyone. For very simple cases, writing programs is more work than using a configuration file. But `freddie` aims to provide a great developer experience, and is the a better option for certain complicated use cases, and besides: LLMs make writing programs a lot easier than before. So, have an LLM do it :)

## Running mercury

Clone the repository. From the root, `cargo run -p mercury` or `cargo install --path crates/mercury && mercury`.

That builds mercury and starts it as a detached daemon, then exits. You can run the following:

```
# these start mercury in the background
mercury
mercury start

mercury restart
mercury stop
mercury status
mercury logs

# install mercury
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

mercury is one consumer of `freddie`, not `freddie` itself. figaro is another, and there will be more. So the test for whether something belongs in mercury is whether figaro would write it differently: if figaro's copy would be identical, it does not belong in mercury, it belongs in a `freddie_*` crate that both depend on.

What mercury keeps is what is only true of mercury: its `App` enum, its state tree, its bindings, its effects, and the table mapping bundle ids onto its apps. What it does not keep is anything about how macOS works. Grabbing the keyboard, foregrounding an app, watching the frontmost app, and giving the main thread to a run loop are all identical in figaro, and each lives in its own crate.

This is easy to get wrong, because the first consumer is the only consumer and everything looks app-specific while it is the only thing there. The rule is about the second consumer, before it exists.

## Planning

Work is planned in `refactors/pending/` and moved to `refactors/past/` once it has shipped. A doc there is meant to carry enough detail that someone with no context can implement it without making design decisions along the way, which is why they hold before-and-after code rather than descriptions of it.

## Prior art

`freddie`'s event loop follows two existing systems. isograph's language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. barnum goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. `freddie`'s difference from barnum is that its handlers mutate state directly during dispatch, where barnum's only return a value the engine writes back. See `refactors/past/event-loop.md` for detail.
