---
title: Dispatch and Precedence
sidebar_position: 4
---

# Dispatch and Precedence

Handlers bound on `NavLayer` take precedence over handlers bound on `Layer`, which take precedence over handlers bound on `Mercury`.

TODO: describe the `ControlFlow` chain that walks the levels and takes the first matching handler, and what "first" means once a level has several bindings.

## Narrowing the event

Dispatch narrows the event to `&Self::Event` with a `TryFrom` before it asks a trigger whether it matches. A key binding never sees a tab event: the narrowing fails and the binding is skipped.

## Double handling

TODO: ideally an event handled twice would be an error. That is not currently enabled in `freddie`; describe what is planned.

## The last resort

`AnyKey` matches every key event there is, and lives at the root as the last resort for whatever no layer claimed.
