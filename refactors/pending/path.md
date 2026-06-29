# Path and resolution

## Model

- One value owns the whole state tree. Paths only borrow it; there is exactly one live `&mut` at a time.
- Enums are structural states. `resolve` descends into the active variant.
- A struct is a leaf, unless it carries a single `#[resolve]` field, in which case `resolve` descends into that field. At most one `#[resolve]` per struct: a struct's fields all coexist, so two would mean two simultaneously-active sub-states, which isn't a single leaf. A choice between sub-states is an enum.
- Plain fields are state. Shared state that several sub-states read (`MouseMode.first_click`) lives on the parent struct and is reached from below via the path. Behavioral flags (`cmd_held`, `caps_lock`) are fields read by the handler, not variants. Orthogonal flags as variants would be a 2^n explosion.
- Exactly one active leaf. A discriminator picks it: an enum's current variant, or a state tag/index over stored children. If the inactive siblings must be preserved across a toggle, store them as fields (with a tag) rather than an enum, which drops the inactive payload. That is still one `#[resolve]`, pointing at the tag, with the preserved state on the parent.
- Make something a variant only when it changes structure (different reachable sub-states). Otherwise it is a field.

## Path (the cursor)

No trait, no associated type, no generic bound. Two inherent methods. The root stores the `&mut` directly with `parent = ()` (it can't use a `project` box, because that box would capture and return the root `&mut`, a lending closure). Every non-root level is a lens (struct field) or prism (enum variant), stored as a box that takes the parent path and returns this node.

```rust
pub struct Root<'a> {
    node: &'a mut Outer,
}
impl<'a> Root<'a> {
    pub fn get_mut(&mut self) -> &mut Outer { self.node }
    pub fn into_parent(self) {}
}

pub struct Path<Node, Parent> {
    parent: Parent,
    project: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>,
}
impl<Node, Parent> Path<Node, Parent> {
    pub fn get_mut(&mut self) -> &mut Node {
        (self.project)(&mut self.parent) // formulaic for every non-root level
    }
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

A multi-parent node is a route enum with its own inherent `get_mut`/`into_parent` matching the route. The `project` box captures any per-level info (an index), so info is never a type parameter. Lens legs (struct fields) are total; prism legs (enum variants) carry the `_ => unreachable!()`.

- Aliasing is prevented statically: `get_mut(&mut self)` borrows the whole path, so the leaf `&mut` and an ancestor `&mut` can't be held at once.
- Staleness (reassign an ancestor, then a stale `get_mut` hits `unreachable!()`) is prevented by making `parent` private and `into_parent` consuming: the only way up moves the path.

## resolve

`outer.resolve()` walks active variants (enums) and `#[resolve]` fields (structs) down to the single active leaf, building the path top-down, and returns `OuterResolvePosition`, an enum of every possible leaf, each carrying its path. No spans, so the walk is a deterministic descent, not a search.

It is an inherent method, not a trait method. Each type's `resolve` calls its children's `resolve` concretely (closed world), so a trait buys nothing. To avoid the aliasing the naive top-down `&mut` walk hits, it matches the discriminant without binding, then moves the `&mut` into the path, then inspects deeper through the path:

```rust
fn resolve(&mut self) -> OuterResolvePosition<'_> {
    match self {
        Outer::Other => OuterResolvePosition::Outer(Root { node: self }),
        Outer::Middle(_) => {
            let mp = /* MiddlePath built from Root { node: self } */;
            mp.resolve_here() // continue descending through the path
        }
        // ...
    }
}
```

## dispatch

Resolve to the leaf, then walk up the path trying each node's bindings. The leaf gets first crack and can override; ancestors supply shared handlers, reached by fallthrough. The path's ascent does double duty: it is both how a transition reaches an ancestor and how dispatch finds a shared handler.

## Derive macro

Per-type `#[derive(Phantom)]`, one per state type, types free to live in separate files of the same crate. No trait; it generates inherent methods. (Same crate is required, not just convenient: the leaf enum names every leaf's path type and `resolve` calls across types, so they must all be visible to each other.)

- Root: `#[phantom(node_type = OuterResolvePosition, is_root)]`.
- Others: `#[phantom(parent_type = MiddleParentType<'a>, node_type = OuterResolvePosition)]`.
- `#[resolve]` on the single descend field of a struct.

The user writes: the state types and their data, the `#[resolve]` marker, the parent enum per node (`MiddleParentType` and friends), the `OuterResolvePosition` leaf enum, and the bindings.

The macro generates: the path alias for each node (`type MiddlePath<'a> = Path<Middle, MiddleParentType<'a>>`), the projection boxes, `get_mut`/`into_parent`, and the inherent `resolve`.

The two things the user must hand-write are exactly the whole-tree facts a per-type derive cannot see from one item: where a node is used (its `parent_type`) and the set of all leaves (`OuterResolvePosition`). That is the cost of per-type derive and of the separate-file layout; a block macro could compute both but would force all the types into one place.

## Open

- The binding grammar: how key->action is declared per state, how an action references a user fn and receives the path, and how field-conditioned bindings (`cmd_held`) are expressed.
