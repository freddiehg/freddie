---
title: Dispatch and Precedence
sidebar_position: 4
---

# Dispatch and Precedence

Handlers bound on `NavLayer` take precedence over handlers bound on `Layer`, which take precedence over handlers bound on `Mercury`.

`#[derive(Bind)]` emits one `dispatch` per node, and they chain through `ControlFlow`:

```rust
fn dispatch<'a>(
    path: NavLayerPath<'a>,
    event: &MercuryEvent,
) -> ControlFlow<Vec<MercuryEffect>, NavLayerPath<'a>>
```

A node descends into its active child before it looks at its own bindings, so the walk goes leafward first and unwinds rootward. `Break` carries the handler's output straight out, past every node still waiting on the way back up. `Continue` carries the path back instead, and the parent recovers its own path from it with `into_parent` and takes its turn. When the root hands back a `Continue`, `bind::dispatch` turns it into `None`, which is what `state.handle` returns for an event nothing bound.

Inside one node the bindings are tried in the order they are written in `#[bind(..)]`, top to bottom, and the first whose trigger matches wins. So "first" is written order within a node and depth between nodes, and exactly one handler runs per event.

## Narrowing the event

Dispatch narrows the event to `&Self::Event` with a `TryFrom` before it asks a trigger whether it matches. A key binding never sees a tab event: the narrowing fails and the binding is skipped.
