# ascend through a derived level

DO NOT DO. A derived level's handler cannot `Ascend`; it reaches ancestors through `parent` instead, and that is the accepted permanent state. Fix A (combinatorial impls) and Fix B (make `Path` a case of `Node`) are both rejected: a `PathMut` addresses a place and a `Node` carries data, so collapsing them puts a mechanism where a payload belongs.

Not in v0. Recorded so nobody retries it blind and hits E0119.

Background is `resolution.md`: a handler is given a `Node<Parent, Data>`, where `Parent` is a `laserbeam::Path` when the level above is a place and a `Node` when it is derived.

## What works today, unchanged

A handler bound at several PLACES names the ancestor it needs and ascends to it.

```rust
fn to_home<'a, P: Ascend<LayerPath<'a>>>(_ev: &KeyEvent, node: Node<P, ()>) -> Vec<MercuryEffect> {
    go_home(&mut node.parent.ascend());
    Vec::new()
}
```

Bound on `Layer` it ascends zero levels; bound on `TypingLayer` it ascends one. The bound moves from the parameter to `Node`'s `parent`, and nothing in laserbeam changes. Compiled.

## What does not work

A DERIVED level's handler cannot ascend. Its `parent` is a `Node`, not a `Path`, and `Node` has no `Ascend` impl.

The obvious impl does not compile:

```rust
impl<T, P, D> Ascend<T> for Node<P, D>
where
    P: Ascend<T>,
{
    fn ascend(self) -> T {
        Ascend::ascend(self.parent)
    }
}
```

```
error[E0119]: conflicting implementations of trait `Ascend<Node<_, _>>` for type `Node<_, _>`
```

It overlaps laserbeam's reflexive impl, `impl<T> Ascend<T> for T`, at `T = Node<P, D>`.

## Why laserbeam's own impls escape this

`Ascend` is generated one impl per depth, and the TARGET APPEARS INSIDE `Self`:

```rust
macro_rules! ascend_nest {
    ($t:ident) => { $t };
    ($t:ident, $head:ident $(, $rest:ident)*) => {
        Path<$head, ascend_nest!($t $(, $rest)*)>
    };
}

impl<T, $($acc,)* $head> Ascend<T> for ascend_nest!(T $(, $acc)*, $head) { .. }

ascend_impls!([], N0, N1, ..., N11);   // twelve depths, twelve impls
```

So depth 2 is `impl<T, N0, N1> Ascend<T> for Path<N1, Path<N0, T>>`. For that to overlap `Ascend<T> for T`, `Self` would have to equal `T`, and `Self` structurally contains `T`. The occurs check rejects it, so there is no overlap.

`Node<P, D>` does not mention `T`. The compiler cannot rule out `P = Node<P, D>`, so it cannot rule out the overlap.

## Fix A: per-depth impls over mixed chains

Generate the same per-depth impls, but let each link be a `Path` OR a `Node`.

The target still appears inside `Self`, so the overlap argument still holds. The cost is combinatorial: at depth `d` each of the `d` links is one of two shapes, so there are `2^d` nests. Covering depths 1 through `D` needs `2^(D+1) - 2` impls. Laserbeam covers 12 depths today with 12 impls; the same reach would need 8190.

Compile time and macro complexity, both for a case v0 does not have.

## Fix B: make `Path` a case of `Node`

The principled one, and it removes the combinatorics rather than paying them.

A node is a parent plus a payload. A PLACE's payload is a projection, and that is exactly what `Path<N, P>` is: `{ parent: P, projection: Proj<N, P> }`. A DERIVED level's payload is data. They are one type with two payloads, not two types.

If `Path<N, P>` were defined as `Node<P, Proj<N, P>>`, every link in every chain would be a `Node`, `ascend_nest!` would generate one shape per depth again, and `Ascend` would reach through derived levels for free.

### What else it deletes

`HasParent`, which exists only because `Path` and `Node` are separate types that both need `into_parent()`.

`Descend`, probably. It exists because a place's walk returns its own path while a derived level's returns its parent, and because the derive cannot name a derived child fn's return type. One node type with one walk collapses the first half of that.

`#[derived_node]`, and this is the one worth naming. It exists only to tell the derive its parent, so that the derive can write `impl Descend<M> for Node<InAppLayerPath<'a>, ChromeInfo>`. With one node type, a derived level's node is named the same way a place's is, `#[laserbeam(path = ChromeNode)]`, and the alias carries the parent inside it exactly as `Path<InAppLayer, LayerPath<'a>>` does today. There is no second attribute to have.

The child side then reads identically for both kinds, and the only surviving difference is the parent side: `#[resolve_into]` for a field, `#[derived_child(f)]` for a function.

### The cost, and why it is deferred

Not size. `Path` could become `type Path<N, P> = Node<P, Proj<N, P>>` with `from_fn`, `get_mut`, `parent`, and `into_parent` preserved, and no existing code would change.

The reason is that a place's payload and a derived level's payload are not the same kind of thing. A projection is a MECHANISM for reaching a value in the tree. Data is a VALUE. Unifying them means a place's `data` is its projection, so `node.data` on a `Path` hands back a function pointer, which is not a payload and should not be presented as one.

The two types are two payloads only in the sense that both sit next to a parent. That is a structural similarity, not a semantic one, and collapsing them buys `Ascend` at the price of a lie in the API.

Deferred. Possibly never.

## v0

Neither. A derived level's handler reaches ancestors explicitly through `parent`:

```rust
gmail.parent.data.tab            // Chrome's data
gmail.parent.parent.get_mut()    // the layer
```

The consequence to accept: a handler cannot be bound at both a place and a derived level, because the two hand it different `Parent` types and only one of them can ascend. Nothing in mercury wants that today.
