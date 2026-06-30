# Rayban — design spec

A self-contained library living in the phantom-kit-2 repo but depending on nothing else in it: the mutable counterpart to isograph's `resolve_position`. From a `&mut Root`, `resolve()` produces a typed cursor (`Rayban`) to the single active leaf of a tree; the cursor reads/mutates the leaf and walks up to ancestors, with exactly one live `&mut` at any time. No `Rc`, no `RefCell`, no `unsafe`. It provides the path types and the `resolve` derive only — no dispatch, no key bindings; those are the consumer's concern.

## Model

- One value owns the whole tree. Cursors only borrow it; exactly one live `&mut` at a time.
- Enums are structural states: `resolve` descends into the active variant.
- A struct is a leaf, unless it has a single `#[resolve_into]` field, in which case `resolve` descends into that field. At most one per struct (its fields coexist, so two would be two simultaneously-active leaves).
- Plain fields are state. Shared state lives on a parent struct and is reached from below via the cursor. Behavioral flags are fields, not variants.
- Exactly one active leaf. A discriminator (enum variant, or a state tag/index over stored children) picks it.

## Runtime types (no trait, no bound)

```rust
// root: holds the one &mut directly; can't be a Rayban because a project box
// would have to capture and return the &mut (a lending closure).
pub struct Root<'a, T> { node: &'a mut T }
impl<'a, T> Root<'a, T> {
    pub fn new(node: &'a mut T) -> Self { Root { node } }
    pub fn get_mut(&mut self) -> &mut T { self.node }
    pub fn into_parent(self) {}
}

// every non-root level. `parent` is PRIVATE (see Staleness). `project` is a lens
// (struct field) or prism (enum variant); it captures any per-level info.
pub struct Rayban<Node, Parent> {
    parent: Parent,
    project: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>,
}
impl<Node, Parent> Rayban<Node, Parent> {
    pub fn new(parent: Parent, project: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>) -> Self {
        Rayban { parent, project }
    }
    pub fn get_mut(&mut self) -> &mut Node { (self.project)(&mut self.parent) }
    pub fn into_parent(self) -> Parent { self.parent }
}
```

A multi-parent node is a route enum the user writes, with its own inherent `get_mut`/`into_parent` matching the route.

- Aliasing is prevented statically: `get_mut(&mut self)` borrows the whole cursor, so the leaf `&mut` and an ancestor `&mut` can't be held at once.
- Staleness (reassign an ancestor, then a stale `get_mut` hits the prism's dead arm) is prevented by `parent` being private and `into_parent` consuming: the only way up moves the cursor, so a stale cursor can't be reused. `Rayban::new` is public so the macro can construct it; `parent` is never readable.

## Resolve (needs a minimal trait)

```rust
pub trait Resolve<'a> {
    type Parent;                 // this node's parent path type (the enum, or the single parent)
    type Resolved;               // the shared enum of all leaves (each carrying its path)
    fn resolve(&'a mut self, parent: Self::Parent) -> Self::Resolved;
}
```

The trait exists only so a parent, when it descends, can name a child's parent-enum type as `<Child as Resolve>::Parent` rather than assuming `ChildParent`. That is exactly how isograph avoids the name magic. The path type itself stays trait-free.

`resolve` walks active variants (enums) and `#[resolve_into]` fields (structs) down to the single leaf, building the cursor top-down, and returns the `Resolved` enum. There are no spans, so it is a deterministic descent. To avoid the aliasing the naive top-down `&mut` walk hits (proven: storing `&mut self` in the cursor while recursing into `&mut child` is E0499), the generated code matches the discriminant without binding, moves the `&mut` into the cursor, then inspects deeper through the cursor.

## Derive

Per-type `#[derive(Resolve)]`, types free across files of one crate (same crate required: the leaf enum names every leaf's path and `resolve` calls across types).

- Root: `#[resolve(resolved = OuterResolved, is_root)]`.
- Others: `#[resolve(parent = MiddleParent<'a>, resolved = OuterResolved)]`.
- `#[resolve_into]` on the single descend field of a struct.

User writes: the state types and data, the `#[resolve_into]` marker, the parent enum per node, the `OuterResolved` leaf enum. The parent enum's variants are named after the parent types (the one naming convention, like isograph).

Macro generates: the projection boxes, and the `Resolve` impl (`resolve`, the associated `Parent`/`Resolved`). It does not generate `FooPath`/`FooParent` names.
