---
title: The Data Model
sidebar_position: 3
---

# The Data Model

The data model is what controls which handler is executed when you call `state.handle(event)`. `mercury` intentionally has a fairly standard one, designed to be extended and modified for your use case.

In the simplest case, the state is a nested enum. `struct Mercury` contains a `#[resolve_into] layer: Layer` field, which is an enum. Different keys can be bound on different layers: `c` navigates to Google Chrome iff `matches!(state.layer, Layer::Nav(_))`, but not in other layers.

## Making impossible states unrepresentable

TODO: the standard this codebase holds itself to, and how the layer enum enforces it.

## Where state lives

TODO: the rule that state lives on the level that uses it, with the volume layer as the example, and what goes wrong when it does not.
