---
title: Getting Started with Mercury
sidebar_position: 2
---

# Getting Started with Mercury

`mercury` is the example program built with `freddie`, and it ships in this repository. It is macOS-only and requires accessibility permissions.

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
mercury install   # start mercury at login
mercury uninstall # stop doing so
```

## Watching what it does

Run `mercury logs` alongside it. Every dispatched event writes one record carrying the event, the effects it produced, and the resulting state.

## The layers

TODO: walk the layer graph — typing, home, nav, inapp, site, resize — and say what each binding does and where it leaves you.

## Where a binding leaves you

TODO: explain the rule that every binding decides which layer it ends in, and that the decision follows from what the user is expected to do next.

## Next

TODO: point at [Implementing Your Own Handler](./implementing-your-own-handler.md) as the first change to make.
