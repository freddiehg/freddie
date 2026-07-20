# `freddie`

**`freddie` is a set of tools for building a bespoke control plane for your computer**.

This program ingests a stream of events and produces a stream of effects. One such event is generated when you press a key on your keyboard, and one such effect is a simulated keypress! **So, `freddie` can be used to build a key remapper.** But the events and effects are arbitrary, and so `freddie` can be used to build something much more powerful.

Want to ensure that when you connect a specific microphone, Wispr Flow uses that one? Want to rearrange your windows when connecting to a specific monitor? Want a keybinding to mute/unmute yourself in Google meets/Zoom? Want a hotkey to send a transcribed message to a specific Claude instance? Want to be able to clone a repository, directly from github.com?

All of this is possible with `freddie`.

Example events include: this key was pressed, this app was foregrounded, this browser tab became active, this external device connected. Example effects include: emit this key, foreground this app, resize this window, run this arbitrary code. A program built on `freddie` is the central place where the decision of how to respond to an event is made.

And `freddie` aims to do this in a way that provides a great developer experience.

This repository contains one such demo program, `mercury`, and you should not expect it to fit your use case. It is here to be read, run, studied, used as an example, forked, and modified. See below for more information.

## Alternatives

### Why `freddie`? Why not karabiner? Why not hammerspoon?

In many ways `freddie` is a replacement for Karabiner and other keyboard remappers. These are excellent programs, but they are limited in their customizability due to being configuration-driven. For example, you can bind keys differently in Karabiner based on which app is foregrounded, but not which Chrome tab is active or which devices are connected. And so, if you want to do that, you have three bad options:

- emit a (hopefully unused) keypress that changes some internal Karabiner state, and make sure to keep that state in sync, or
- bind all keys for all states, and then have the handler know what to do, or
- use an external program, such as hammerspoon.

Most folks will choose the third option, leading to a spaghettification of configuration code, and a difficulty reasoning about the overall state.

### What alternative is there to being configuration-driven?

With Karabiner, you download a binary and provide a configuration. With `freddie`, you fork the repository, make the changes you want, and run `cargo build` to generate the new binary.

That gives the freedom to do whatever you want: you can respond to whatever events you want, and you can manage state however you choose, and your handlers receive this state.

This comes at a cost. For very simple cases, writing programs is more work than using a configuration file. But `freddie` aims to provide a great developer experience, and is the a better option for certain complicated use cases, and besides: LLMs make writing programs a lot easier than before. So, have an LLM do it :)

## `mercury`

This repository contains a sample program built with `freddie`, entitled `mercury`. You should not expect it to fit your use case. It is here to be read, run, studied, used as an example, forked, and modified.

### Running `mercury`

`mercury` is the example program, built with `freddie`, and is included in this repository.

To install it, clone the repository. From the root, `cargo run -p mercury` or `cargo install --path crates/mercury && mercury`.

That builds mercury and starts it as a detached daemon, then exits. You can run the following:

```
# these start mercury in the background
mercury
mercury start

mercury restart
mercury stop
mercury status
mercury logs

# start mercury at login, and stop doing so
mercury install
mercury uninstall
```

`mercury` is macOS only, and it needs Accessibility permissions.

Max one instance of `mercury` runs at a time.

### The tour

mercury comes up as a menu-bar item. The icon shows the layer you are in, and Quit lives in that menu deliberately: the way out must not depend on the grabbed keyboard still working.

It boots into typing, which binds nothing. Every key falls through to the root, runs through the `jk` sequence, and passes through, so the keyboard is normal until you ask for something.

Type `jk` quickly, inside 200ms, and you are in home. Home is a command layer: it swallows keys it does not bind, which is why it cannot be what you boot into. From there:

- `n` then `c` foregrounds Chrome. `f`, `g`, `z` do the same for Finder, Ghostty, and Zed, and each lands you in that app's own layer once it is actually frontmost.
- `r` then an arrow places the focused window: up maximizes, left and right take half the screen. Then it returns home on its own.
- `o` shows the current layer's keymap as an overlay, so you do not have to remember any of this.
- `t` returns to typing, `escape` returns to home from every layer that binds it, and `q` quits.

Once Chrome is frontmost, `r` refreshes the tab. Once Ghostty is, `j` and `k` walk tmux's windows and `1` through `0` jump to the first ten. Those bindings exist only while that app is up. Go one level finer and the frontmost URL matters too: on `claude.ai`, `n` starts a new chat.

### The model

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

### The edges

Inside is pure. Outside is arbitrary code, and that is the point.

Sources are whatever can produce an event: the keyboard grab, the frontmost-app watcher, and the menu-bar item are the ones built on macOS APIs. Watching a file and emitting an event when it changes would be another. Effects are the same in reverse.

mercury also listens on `127.0.0.1:3883`, so processes outside it can push events in. Anything that can hold a WebSocket can drive it:

```
{"kind":"IncomingEvent.Tab","value":{"url":"https://claude.ai/new"}}
```

`chrome-extension/` is the first client, pushing the front tab's URL, which is how the site layer knows where you are.

`IncomingEvent` is the whole external vocabulary, so a sender can report the front tab and nothing else. `MercuryEvent` does not derive `Deserialize`, which is what makes remote key injection and remote quit unrepresentable rather than filtered. Web pages are refused at the handshake, because a WebSocket handshake is exempt from the same-origin policy and any open tab could otherwise reach the socket.

### Logs

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

## Roadmap

Work is haphazardly planned in `refactors/pending/` and moved to `refactors/past/` once it has shipped.

## Contributing

Please reach out! `freddie` is moving very fast, so my fear is that, in the amount of time it takes to coordinate on the right work, I can just ask my clanker to implement the feature. But I'd love to hear about what you want to see in `freddie`.

But it should (by and large) be ready for folks to experiment with!

## Prior art

`freddie`'s event loop follows two existing systems. [`isograph`](https://github.com/isographlabs/isograph)'s language server is the same shape: several sources feed one queue, one event is dispatched per iteration, and dispatch is a `ControlFlow` chain that takes the first matching handler. [`barnum`](https://github.com/barnum-circus/barnum) goes a step further with deferred effects run off a queue by an async scheduler, whose results feed back as events. `freddie`'s difference from `barnum` is that its handlers mutate state directly during dispatch, where `barnum`'s only return a value the engine writes back. See `refactors/past/event-loop.md` for detail.
