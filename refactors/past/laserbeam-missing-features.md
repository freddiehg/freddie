# laserbeam: missing features

Superseded. This describes the old `#[derive(Laserbeam)]` and its `resolve()` machinery, which are deleted; `crates/laserbeam_macro` no longer exists. The tree-shape concerns (enum descent, multi-parent, generic nodes) are now bind's, in `crates/bind_macro` and `crates/derive_support`.

What `#[derive(Laserbeam)]` does not handle yet, grouped by how it fails. Each entry notes whether it is a fundamental constraint or just unimplemented, so we know what is cheap to add. This reflects the macro in `crates/laserbeam_macro/src/lib.rs` as of now.

## Hard rejections

These produce a clear compile error from the macro itself.

### Generic nodes

`node_impl` rejects any node with generic parameters: "laserbeam nodes may not be generic". The path alias is emitted as `#p<'a>`, threading exactly one lifetime and no type params, so a node like `struct Single<T> { .. }` cannot derive. The path alias also cannot carry an unused type parameter (Rust's E0091), so you cannot smuggle a type in that way either. Monomorphize the node instead.

Status: fundamental to the current single-lifetime path shape. Supporting generic nodes means changing `#p<'a>` and the rejection guard together.

### Enum variants that are not single-field tuples

`single_field_ty` requires `Foo(Bar)`. Unit variants (`Foo`), struct variants (`Foo { x: T }`), and multi-field variants (`Foo(A, B)`) all error with "expected a single-field tuple variant `Foo(Bar)`".

Status: easy to extend for struct/multi-field variants if a use case appears; unit variants have nothing to descend into, so they would have to be treated as leaves.

### More than one `#[resolve_into]` per struct

A struct descends into exactly one child. Two `#[resolve_into]` fields error with "at most one `#[resolve_into]` field per struct". Branching is expressed through enums, not through a struct with multiple children.

Status: this restriction must be lifted. We absolutely need multiple `#[resolve_into]` per struct, because a node can have several concurrently-active children, not one. The reactive-UI case forces it: a page showing a blog and an open dropdown has both active and subscribed at once. mercury needs it directly too: a laptop-keyboard layer and an external-keyboard layer are active simultaneously and not necessarily in the same state (the laptop in nav while the external is in typing). That second motivation is not buildable today, and the doc should not lean on it: a `CGEventTap` cannot tell which physical keyboard a key came from. The event carries `KEYBOARD_EVENT_KEYBOARD_TYPE`, a model class rather than a device identity, and on this machine Karabiner already collapses every physical keyboard into one virtual HID device upstream of our tap. Per-keyboard layers wait on virtual-hid.md, where the device is the thing you read from rather than a field on an event. The reactive-UI motivation stands on its own. This drops the single-active-leaf assumption, so `resolve` becomes multi-valued (a set of active leaves), accumulation unions bindings across all of them, and only dispatch of a single fired event is still one path. The distinction from enums: an enum is an exclusive fork (one active variant), multiple `#[resolve_into]` is an inclusive fork (all marked children active at once).

## Silent gaps

These compile, but the generated code fails to type-check, so the user gets a confusing downstream error rather than a diagnostic from the macro. These are the real holes.

### Collections and optionals as the descent target

The macro descends into exactly one child: for an enum, the active variant; for a struct, its one `#[resolve_into]` field. There is no "which of N children is active" or "is the child present" logic. A field typed `Vec<Child>` or `Option<Child>` is passed through as the child type (only `Box` is unwrapped), so the generated `<Vec<Child> as Resolve>::resolve(..)` does not compile.

This is the biggest gap for a real tree. isograph handles it: `resolve_position_macro` recognizes `Vec<T>` and `Option<T>`, iterates, and uses `span.contains(position)` to pick the active child. laserbeam has no equivalent active-child selection, because mutably it would need a way to say which index/variant is live.

Status: a genuine missing feature, not a rough edge. Needs an "active child" mechanism (an index, a predicate, or an explicit cursor field) plus iteration in the generated descent. Scoped in `laserbeam-state-controlled-children.md`.

### Indirection other than `Box`

`unbox` only matches `Box<T>`. A recursive or shared field typed `Rc<Child>`, `Arc<Child>`, or `&Child` is not dereferenced, so the projection produces the wrong type.

Status: easy to extend `unbox` to other smart pointers, but `Rc`/`Arc` do not give `&mut` without `get_mut`/`make_mut`, so the mutable story for those is not just a deref. `Box` is the only one that is a clean `&mut` deref.

### Tuple-struct nodes that need to descend

`find_resolve_into` only scans `Fields::Named`. A tuple struct `struct Foo(Bar)` cannot mark its descent field. A tuple-struct leaf (no descent) is fine, since leaves do not look at fields.

Status: easy to add (index the unnamed field) if a tuple-struct node ever needs `#[resolve_into]`.

## Shape constraints

These work, but the surrounding types must be written a specific way; a mismatch is a compile error.

### `resolved` must be an enum, variant-per-leaf

The leaf body emits `#resolved::#name(path)`, so `resolved` must be an enum with a variant named exactly after each leaf type (`Resolved::Credit(..)`). It cannot be a plain struct. There is a `POTENTIAL` note in `struct_body` to emit `path.into()` instead, which would let `resolved` be a struct with a `From<LeafPath>` impl.

### Route enums must have a variant named after each parent

Both the struct and enum multi-parent descents construct `#route::#name(path.into())`, where `#name` is the parent node's own identifier. The route enum must therefore have a variant named exactly after each parent node (`AskToLetGoParent::RefuseToLetGo`, `TrackParent::Discography`). A mismatch is a compile error. The variant payload type is the parent's path (or `Box<ParentPath>` for a recursive chain); `path.into()` adapts via `From<T> for T` or `From<T> for Box<T>`.

### Path aliases: one lifetime, no type params

A path alias is referenced as `#p<'a>`, so it must take exactly one lifetime and nothing else. This follows from the generic-node rejection above.

## What is supported (for contrast)

So the boundaries are clear, the following all work and are covered by tests:

- Single-parent and multi-parent nodes, the multi-parent parent expressed as a route enum in the `Parent` slot (`Path<Node, ParentEnum>`).
- Recursive data, broken with `Box` in a struct field, an enum variant, or a route-enum variant; the macro dereferences each.
- Multi-parent descent from a struct field, a non-root enum variant, and a root enum variant.
- Re-resolving from a mid-tree path after mutating which branch is active.

## Future: type-level enumeration of all states

A mode that enumerates, at the type level, every leaf the tree can resolve to. `enum MediaType { Album(Album), Single(Single) }` yields an iterator over each reachable leaf state, derived from the types rather than walked from a runtime value.

- Not v1.
- Use: with the freddie bindings, validate every possible state (each state's accumulated bindings are well-formed, no required binding missing) and generate documentation of the full state space.
- It does not compose with the dynamic resolve_into (state-controlled children) feature, at least not without more thought. The domain of a dynamic descent, which children can exist, is a runtime value, so the reachable leaves under it are not statically known. Type-level enumeration needs the leaf set statically determined; the enum and `#[resolve_into]` cases give that, a state-selected collection does not. See `laserbeam-state-controlled-children.md`.
