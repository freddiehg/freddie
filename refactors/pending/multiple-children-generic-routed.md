# Multiple children: generic and routed edges

Downstream of `multiple-children.md`, whose change 3 rejects both of these combinations with a derive error, in the style of today's `reject_routed_generic`:

- "a generic child may not share a node with other children"
- "a routed child may not share a node with other children"

This doc is the work that lifts those two errors. It is not actively worked; it exists so the rejection is a recorded decision with a path out, not a dead end. Nothing in mercury or figaro hits either error: the only generic children in either tree are the typing dolls, which `multiple-children.md`'s flattening deletes, and no routed edge sits beside a sibling.

## The generic edge

For a single generic child, the derive emits the path-equality bound that makes the opaque parameter concrete enough for the generated body's inherent calls to resolve (`generic-doll-nodes.md`):

```rust
where
    Next: 'static + ::bind::Dispatch<M>,
    for<'q> Next: ::laserbeam::HasPath<
        Path<'q> = ::laserbeam::PathMut<Next, <Self as ::laserbeam::HasPath>::Path<'q>>,
    >,
```

`child_where_clause` already iterates a list of children (enum nodes have one per variant), so the multi-child emission is the same predicate once per generic child, and the `descend` block's closure builds the child path with the same `PathMut::from_fn` the single-child body normalizes through. The expected result is that nothing new is emitted at all; what is missing is the verification, to the standard the single-child case got before it landed: a standalone mock, compile-checked end to end, covering

- a branch point with one generic child beside concrete children,
- a branch point generic over two children, `Shell<A, B>`, each a `#[child]` field,
- both parameters instantiated to the same type (the two-keyboards shape, `Shell<K, K>`),
- dispatch round-tripping through a `descend` block whose closure resolves inherent methods through the normalized `Path`.

If the mock passes, the lift for this edge is deleting the rejection and pinning the mock as a bind test beside `generic_shell.rs`.

## The routed edge

A routed (multi-parent) child's fold is not the plain `Stop` conversion; it is a hand-written match on the consumer's two enums, recovering the variant this descent constructed (`child_state`'s route arm). Inside a `descend` block the same match moves into the closure, which still returns the uniform `MaybeInvalidated<PPath>`:

```rust
state = state.descend(|p| {
    let child_path = /* the route wrap child_state builds today, over `p` */;
    match C2::dispatch(child_path, event, effs, claim).into_inner() {
        Stop::Here(child) => {
            let RouteEnum::Parent(recovered) = child.into_parent() else {
                ::core::unreachable!()
            };
            MaybeInvalidated::NotInvalidated(recovered)
        }
        Stop::Up(above) => {
            let UpEnum::Parent(completed) = above else { ::core::unreachable!() };
            MaybeInvalidated::Invalidated(completed)
        }
    }
});
```

The composition question is narrow, because `descend`'s own internals never see the route enums: they are unwrapped inside the closure, and `descend` handles only `MaybeInvalidated<PPath>`. What needs writing before the rejection lifts:

- this expansion, compile-checked in a mock with a routed child beside a plain sibling;
- the check half under the `check` feature, whose accumulate recover does the same variant match;
- the routed-and-generic interaction stays rejected (`reject_routed_generic` is untouched; a routed child has no generic story at any arity).

## The lift

Both halves land the same way: the mock proves the expansion, the mock becomes a bind test, and the corresponding rejection in `bind_macro` is deleted. Neither half blocks the other; either can lift alone.
