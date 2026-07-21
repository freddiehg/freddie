---
slug: /
title: Introduction
sidebar_position: 1
---

# Introduction

freddie is a set of tools for building a bespoke control plane for your computer.

A freddie program ingests a stream of events and produces a stream of effects. One such event is generated when you press a key on your keyboard, and one such effect is a simulated keypress, so freddie can be used to build a key remapper. But the events and effects are arbitrary, and so freddie can be used to build something much more powerful.

Want to ensure that when you connect a specific microphone, Wispr Flow uses that one? Want to rearrange your windows when connecting to a specific monitor? Want a keybinding to mute yourself in Google Meet or Zoom? Want a hotkey to send a transcribed message to a specific Claude instance? Want to clone a repository directly from github.com? All of this is possible with freddie.

## Events and effects

Example events include: this key was pressed, this app was foregrounded, this browser tab became active, this external device connected. Example effects include: emit this key, foreground this app, resize this window, run this arbitrary code. A program built on freddie is the central place where the decision of how to respond to an event is made.

## Why not Karabiner, why not Hammerspoon?

Karabiner and the other remappers are excellent programs, but they are configuration-driven, and that bounds what they can express. You can bind keys differently based on which app is foregrounded, but not on which Chrome tab is active or which devices are connected. So you either smuggle state through unused keypresses, bind everything everywhere and sort it out in the handler, or reach for an external program like Hammerspoon. All three spread the configuration out and make the overall state hard to reason about.

With freddie, you fork the repository, make the changes you want, and run `cargo build` to get a new binary. You respond to whatever events you want, you manage state however you choose, and your handlers receive that state.

That costs more than a config file for very simple cases. It wins for the complicated ones, and LLMs make writing the program a lot cheaper than it used to be.

## `mercury`

This repository ships one program built on freddie, called `mercury`. It is macOS-only and it is the working example: read it, run it, study it, fork it, modify it. You should not expect it to fit your use case.

[Getting Started with Mercury](./getting-started-with-mercury.md) walks through installing and driving it.

## Where to go next

- [Getting Started with Mercury](./getting-started-with-mercury.md) runs the example program.
- [Implementing Your Own Handler](./implementing-your-own-handler.md) writes the first binding.
- [Connecting a New Source of Events](./connecting-a-new-source-of-events.md) adds a new event.
- [Adding an Effect](./adding-an-effect.md) adds a new thing the program can do.
- [Architecture](./architecture/index.md) explains how dispatch works.
- [Interacting with macOS](./interacting-with-macos/index.md) covers the platform layer.
