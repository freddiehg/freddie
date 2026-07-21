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

## Double handling

freddie has the check that would catch it, and `mercury` does not run it.

`bind::accumulate` walks the same active tree `dispatch` walks and inserts every live binding's trigger into a `HashSet`, returning `BindError::DuplicateTrigger` when the trigger is already there. It sits behind `bind`'s `check` feature, which is a test-only thing: `mercury` takes `bind` with `default-features = false` and turns `check` back on through a dev-dependency, so a shipped binary contains no `accumulate` at all. A clobber is a property of the program rather than of a run, so a test sees everything a running binary would, earlier.

Two holes keep it from being the guarantee it looks like. Triggers are compared as they are written, so `Key::KeyR` and `Key::KeyR.down()` are two entries and neither is reported, and `AnyKey` is one value that matches every key event while colliding with nothing. Closure triggers are skipped outright, because their value is read out of the state at dispatch rather than claimed statically.

Closing those is planned, and none of it is built:

- A trigger gains `fn expand(self) -> Vec<Trigger>`, the concrete triggers it claims, and the set holds those rather than triggers as written. `is_matching` stays the specification, and `expand` is tested against it over every trigger and every event.
- `AnyKey` gains an except list and expands through a `Key::ALL` generated from the same declaration as the key enum, so a catch-all becomes an ordinary binding that visibly claims what it claims.
- Press joins the concrete keyboard trigger, so `Key::KeyR` and `Key::KeyR.down()` stop shadowing each other silently.
- Each binding declares its mode, on the binding doing the shadowing. No-clobber is the default and a collision is an error. `expects_clobber` says the shadowing is deliberate, and then shadowing nothing is the error, because an override written to beat a binding that has since moved still fires and no behavioral test goes red.
- The check runs over every reachable state, which needs those states enumerated.

## The last resort

`AnyKey` matches every key event there is, and lives at the root as the last resort for whatever no layer claimed.
