---
title: Getting Started with Mercury
sidebar_position: 2
---

# Getting Started with Mercury

`mercury` is the example program built with freddie, and it ships in this repository. It is macOS-only and requires accessibility permissions.

## Installing

```bash
git clone https://github.com/freddiehg/freddie
cd freddie
cargo install --path crates/mercury
mercury
```

`mercury` builds the binary, starts it as a detached daemon, and exits.

## The verbs

```bash
mercury           # start one in the background
mercury start     # the same thing, spelled out
mercury restart   # replace the running one
mercury stop      # end it through the model
mercury status    # report the running one and its pid
mercury logs      # follow the log
mercury install   # optional: start mercury at login
mercury uninstall # optional: stop doing so
```

`install` and `uninstall` are optional. Nothing else needs them: `mercury` runs the same either way, and the only thing `install` changes is that macOS starts one for you at login instead of you typing `mercury`.

## Watching what it does

Run `mercury logs` alongside it. Every dispatched event writes one record carrying the event, the effects it produced, and the resulting state.

As of this writing it boots into this state:

```
Mercury { foreground: Foreground { app: Ghostty, navigating: false }, typing_state: TypingState { held: HeldModifiers {}, jk: KeySequence {} }, overlay: None, layer: Typing(TypingLayer) }
```

Read that closely and you can see it booted into the typing layer. The menu bar item says the same thing, showing a mercury icon and the word "Typing".

## The layers

In the **typing** layer, every keystroke passes through. The only way out is the sequence `jk`, which takes you to the home layer. Pause for 200ms after the `j` and you get to type the characters `jk` like anyone else.

From any layer except typing, `o` shows an overlay of what is bound. Press it from **home** and you will find `n` for nav, `t` for typing, `i` for inapp, `s` for site, `r` for resize, and `q` for quit.

From the **nav** layer, `t` returns to typing, `z` foregrounds Zed, `f` Finder, `g` Ghostty, `c` Google Chrome, and `space` opens Spotlight. `esc` goes home, as it does from every non-typing layer. These are not the apps you use, so fork it.

The **inapp** layer, `i` from home, binds keys per foregrounded app. In Chrome, `r` refreshes, `l` selects the location bar, `shift-l` copies the location, and `cmd-l` copies just the host, turning `https://www.x.com/foo` into `www.x.com`. Other apps bind other things; the source is the list.

The **site** layer, `s` from home or from inapp, binds keys per site, and needs the Chrome extension at `./chrome-extension` loaded so mercury knows which tab is active. On `claude.ai`, `n` opens a new chat, normally `cmd-shift-o`.

In the **resize** layer, `r` from home, `up` maximizes the focused window, `right` takes the right half, and `left` the left half.

The menu bar item shows the current layer and offers a quit option. If you iterate your way into an unresponsive keyboard, that is how you get out.

## Next

[Implementing Your Own Handler](./implementing-your-own-handler.md) is the first change to make.
