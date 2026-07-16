# a custom resolve_into that can decline to descend

Not built. A `#[resolve_into]` (a struct field, or an enum's active variant) always descends into its child. This is the way to make descent CONDITIONAL: let the node decide at dispatch time whether it has an active child, or is itself the leaf. The general "sometimes there is an active child, sometimes not" case, whose concrete consumer is the homogeneous collection in `laserbeam-state-controlled-children.md`.

## What resolve_into does

`#[resolve_into]` names the child that dispatch descends into. On a struct it is a field; on an enum it is the active variant. Either way the derive generates a projection, a `fn(&mut Node) -> &mut Child`, and dispatch follows it down to the child, runs there, and walks back up. Two properties define it:

- It is a PROJECTION into real, persisted state, so the child is mutable and edits stick. (This is what separates it from a read-only derived child, whose edits to a copy would not stick.)
- It is UNCONDITIONAL and FIXED. A struct's `#[resolve_into]` field is always descended into; there is no "descend only when." The child is chosen by the code the derive writes, from a field name or a variant tag, not by anything the node decides at dispatch time.

## Can a custom function do it?

Yes. Nothing about descent requires the projection to be derived from a field. The node could supply it:

```rust
#[custom_resolve_into(pick)]
struct Mercury { /* ... */ }

fn pick(m: &mut Mercury) -> &mut Layer { &mut m.layer }
```

The derive stops generating the projection and calls `pick` instead. This alone is not interesting (it just moves the same `&mut Self -> &mut Child` into user code), but it opens the door, because a function can do things a field access cannot.

## And it can return Option, which is the whole point

The function can return `Option<&mut Child>`:

```rust
fn pick(node: &mut Node) -> Option<&mut Child> {
    node.has_active_child.then(|| &mut node.child)
}
```

`Some(child)` means descend into it; `None` means do not, and this node is itself the active leaf. That is conditional descent, gated on plain state, with the child a plain field: no enum wrapping it just to express "sometimes absent," and no state moved between arms. When `pick` returns `None`, resolve stops at this node and its own binds are the active set.

The same mechanism covers every "sometimes there is an active child, sometimes not" case: a browser with nothing focused, a collection with no selection, an optional sub-mode. The node returns `None` and handles the event itself.

## What it needs from laserbeam

This is a real change, and the semantics are already worked out in `laserbeam-state-controlled-children.md`, which describes the same `fn(&mut Node) -> Option<&mut Child>` hook under `#[custom_resolve_into_fn]`, plus the collection/index sugar built on top of it. The load-bearing pieces:

- Fallible resolve. `resolve` today always lands on exactly one leaf. With a custom fn that can return `None`, a node can be interior and a leaf at once, so `Resolved` gains a node-as-leaf variant and the generated descent has two arms: `Some` builds the projection and recurses, `None` stops here.
- The projection is a `Proj::Dyn` (a boxed closure), which `Path::from_box` already constructs, so the runtime supports it; the missing part is the derive recognizing the attribute and emitting the fallible descent.

`option-resolve-into.md` is the neighboring, smaller special case: a `#[resolve_into]` field whose TYPE is `Option<Child>`, absent when `None`. That is the same conditional descent without a custom function, for when the option lives in the data rather than in a decision.

## Open questions

- The attribute's surface: one hook `fn(&mut Node) -> Option<&mut Child>`, or also the selector sugar (`fn(&Node) -> Option<Index>`) from `laserbeam-state-controlled-children.md`. Likely both, the full projection as the escape hatch.
- Whether to unify with `option-resolve-into.md` (the field-`Option` case) under one code path, since both emit a fallible descent that lands on the node when there is no child.
- Naming: `#[custom_resolve_into]` vs `#[custom_resolve_into_fn]` vs a `resolve_with` attribute.
