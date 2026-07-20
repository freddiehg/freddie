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

This repository contains a sample program built with `freddie`, entitled `mercury`. It is MacOS-only, and requires accessibility permissions. At most, one instance of `mercury` runs at a time.

You should not expect it to fit your use case. It is here to be read, run, studied, used as an example, forked, and modified.

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

### `mercury` user guide

I would recommend you, in addition to starting `mercury`, run `mercury logs`. This will allow you to see the state after every event. As the commit of this writing, it boots up into this state:

```
Mercury { foreground: Foreground { app: Ghostty, navigating: false }, typing_state: TypingState { held: HeldModifiers {}, jk: KeySequence {} }, overlay: None, layer: Typing(TypingLayer) }
```

If you read that state closely, you'll see it booted up into the typing layer. You can also see this by examining the menu bar item, which should sho a mercury icon and the string "Typing".

In this **typing** layer, all keystrokes are passed through. The only way to leave the typing layer is to enter the sequence `jk`, which navigates to the home layer. (If you pause for at least 200ms after typing `j`, you will be able to type the characters `jk`.)

From any layer except the typing layer, you can press `o` to to show an overlay. If you now press it from the home layer, you'll find that you that you can press `n` for nav, `t` for typing, `i` for inapp, `s` for site, `r` for resize and `q` for quit.

From the **nav** layer, you can hit `t` for typing, `z` to foreground zed, `f` to foreground finder, `g` to foreground ghostty, `c` to foreground Google Chrome, `space` to open spotlight, and `esc` to go home (all non-typing layers send you home after you type `esc`). (These aren't the apps you use?? Fork it!)

When an app is foregrounded, if you enter the **inapp** layer (`i` from home), you'll have keybindings that are custom to that app. In Chrome, `r` refreshes, and `l` selects the location bar, `shift-l` copies the location, `cmd-l` copies just the host (i.e. `www.x.com` from `https://www.x.com/foo`). (This has other behavior for other foregrounded apps, see the source code.)

There is also a Chrome extension (at ./chrome-extension) that you can load into Chrome, which will report the URL of the foregrounded tab. If you do this, then the **site** layer (accessible via `s` from home or from inapp) will have per-site bindings. For example, on `claude.ai`, `n` will create a new tab (normally bound to `cmd-shift-o`).

In the **resize** layer (`r` from home), `up` maximizes a window, `right` resizes to the right half, `left` resizes to the left half.

And in addition, mercury creates a menu bar item, which shows the current layer name and exposes a "quit" option. If you, while iterating, end up with a non-responsive keyboard, you can still save yourself :)

## Architecture of a `freddie` program

### Big picture

Every `freddie` app, including `mercury`, will have a similar model, which should be familiar to those acquainted with the [elm architecture](https://guide.elm-lang.org/architecture/). A program maintains some state and receives a stream of events. These events are dispatched, which may result in a handler getting called. Which specific handler is executed depends on the program state. A handler can mutate the state and return effects. These effects are handled.

The "regular program" part is the setup: subscribe to streams of events and turn those into an enum, and call `let effects = state.handle(event).unwrap_or_default()`, and then for each effect, handle it.

The "freddie" part of the program is everything that happens when you call `state.handle(event)`. Conveniently, `handle` is a pure state transformer: you pass the state and event in, and you receive the updated state and effects out. This makes it extremely easy to test!

> To be pedantic: `state.handle` can also create timer effects (which bump a global ID), and dropping the corresponding timer guard prevents those timer effects from firing. But because we don't execute effects in tests, including timer effects, `state.handle` remains pure and testable. Likewise, we don't assert anything about timer IDs.

With that out of the way, let's discuss the specifics of `mercury`.

### `mercury` setup

`mercury` uses `clap` to parse commands. The main subcommand is `mercury start`, which calls a hidden internal command, `mercury daemon`, which does the following:

- Takes the single-instance lock, so multiple instances of `mercury` cannot run at the same time.
- Creates the initial state.
- Puts up the menu bar item.
- Grabs the keyboard, which swallows every key and hands it to the model as an event. The grab also hands back an emitter, which is how keys get back out.
- Subscribes to the other sources: the frontmost app, the event socket on `127.0.0.1:3883`, and SIGTERM.
- For each event, calls `state.dispatch(event)`, which gives us a vector of effects.
- For each effect, handles it. For example, doing so might emit a keypress, foreground an app, change the menu bar text, or quit mercury.

### `mercury` data model

The `mercury` data model is what controls which handler is executed when you call `state.handle(event)`. `mercury` intentionally has a fairly standard data model, designed to be easily extended and modified for your use case.

In the simplest case, the state is a nested enum. For example, `struct Mercury` contains a `#[resolve_into] layer: Layer` field, which is an enum. Different keys can be bound on different layers. For example, `c` navigates to Google Chrome iff `matches!(state.layer, Layer::Nav(_))`, but not in other layers.

Here, the handlers bound on `NavLayer` take precedence over the handlers bound on `Layer`, which take precedence over the handlers bound on `Mercury`. (Ideally, we would like to error if an event would be handled twice, and that is on the roadmap.)

However, this runs into a limitation! How do you handle the currently foregrounded app, which is only relevant in in the `InApp` layer? On the other hand, `struct InApp` could have `#[resolve_into] currently_foregrounded_app: CurrentlyForegroundedApp`, and that would work! But, that means that when you navigate to the inapp layer, you must know (or discover) the foregrounded app.

Discovering it at that time is not a great pattern. Learning what app is foregrounded is quick, so in this specific case, it wouldn't be a problem. But, what if it wasn't so easy? For example, if finding out meant making a network request? Regardless, doing so is impure, and thus violates one of the basic tenets of `freddie`: `state.handle` is pure.

So, we must know what app is foregrounded. Hence, the root `Mercury` struct keeps track of what app is foregrounded. But now, how to populate `#[resolve_into] currently_foregrounded_app`? Do we copy the state when transitioning? That works, but it also means that we have to be careful and not only maintain the state at the root, but also keep the state wherever it is used up-to-date.

The solution that `freddie` offers is virtual fields.

A virtual field is a child level that is computed during dispatch instead of stored in the state. `AppLayer` declares one with `#[derived_child]`, naming the function that builds it:

```rust
#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[derived_child(app_data)]
#[bind(
    Key::Escape.down() => to_home,
    Key::KeyO.down() => toggle_overlay,
    // ...
)]
pub struct AppLayer {
    home_timeout: TimerGuard,
}

/// Reads the confirmed front app, the only copy, and builds the level for it.
const fn app_data(path: &AppLayerPath) -> Option<AppData> {
    // AppLayer -> Layer -> Mercury.
    let root = path.parent().parent();
    match root.foreground.confirmed() {
        Some(App::Chrome) => Some(AppData::Chrome(ChromeApp::new())),
        Some(App::Ghostty) => Some(AppData::Ghostty(GhosttyApp::new())),
        _ => None,
    }
}
```

When dispatch reaches `AppLayer`, it calls `app_data`, which walks up to the root, reads the one copy of the frontmost app, and hands back the level to descend into. `ChromeApp` and `GhosttyApp` are `#[derived_node]`s: they are not stored anywhere and they hold nothing, existing for the length of one dispatch to carry their bindings. So `r` refreshes only while Chrome is frontmost, `j` and `k` walk tmux's windows only while Ghostty is, and the apps that bind nothing get `None` and no level at all.

Nothing is copied on the way into the layer, so nothing can go stale. `app_data` receives a shared reference, so a virtual field reads the state and cannot write it or reach out to the world, which keeps `state.handle` pure.

The same attribute is how the site layer reaches per-site bindings, off the URL the Chrome extension reports.

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
