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

## freddie and `mercury`

freddie is a library. It has no keybindings, no opinion about what a layer is, and nothing to run. What it gives you is the machinery: a way to declare state, bind triggers to handlers on it, and dispatch an event through that state to the handler it selects.

`mercury` is a program written with that library, and it lives in this repository. Its layers, its keymap, its event enum and its effect enum are all its own. None of it is freddie's, and none of it is a default you inherit.

So there are two things you can read here, and they answer different questions:

| | freddie | `mercury` |
| --- | --- | --- |
| What it is | the library | one program using it |
| Decides | how dispatch works | what your keys do |
| You use it by | depending on it | forking it, or ignoring it |

`mercury` is macOS-only and requires accessibility permissions. It exists to be read, run, studied, forked, and modified. **You should not expect its bindings to suit you.** They are one person's, and the point of the exercise is that yours are yours.

Reading it is still the fastest way to learn freddie, because every freddie concept appears in it somewhere concrete. That is why the pages that follow teach with `mercury`'s code rather than with invented examples.

## Where to go next

- [Getting Started with Mercury](./getting-started-with-mercury.md) runs the example program.
- [Implementing Your Own Handler](./implementing-your-own-handler.md) writes the first binding.
- [Connecting a New Source of Events](./connecting-a-new-source-of-events.md) adds a new event.
- [Adding an Effect](./adding-an-effect.md) adds a new thing the program can do.
- [Architecture and Best Practices](./architecture/index.md) explains how dispatch works and how to test it.
- [Interacting with macOS](./interacting-with-macos/index.md) covers the platform layer.
