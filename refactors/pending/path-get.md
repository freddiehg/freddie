# a path you can read without a unique borrow

`PathMut` addresses a node, and reading that node needs `&mut self`:

```rust
pub fn get_mut(&mut self) -> &mut Node
```

Not because reading writes anything, but because the only projection a path stores goes one way:

```rust
enum ProjMut<Node, Parent> {
    Bare(fn(&mut Parent) -> &mut Node),
    Dyn(Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>),
}
```

Both forms want `&mut Parent`, and `&self` cannot produce one. So a shared `&PathMut` can hand back `parent()`, which is a field, and nothing else. Anything that only reads still takes the whole path uniquely, and a node borrow blocks walking up while it lives.

A path stores the shared projection beside the mutable one, so `get(&self) -> &Node` exists. Then a reader takes `&PathMut`, reads its node and its parent at once, and cannot write either.

The immediate consumer is a trigger closure (`refactors/past/trigger-closures.md`): it is handed the node's own struct today, precisely because a shared path could not produce one, which leaves a trigger unable to read anything above itself.

## change 1: paths carry both projections

`crates/laserbeam/src/lib.rs`, beside `ProjMut`:

```rust
/// How a path re-derives its node from its parent, for reading.
///
/// The mirror of [`ProjMut`], and stored beside it: a projection that only goes one way makes
/// every reader take the path uniquely.
enum ProjRef<Node, Parent> {
    Bare(fn(&Parent) -> &Node),
    Dyn(Box<dyn for<'p> Fn(&'p Parent) -> &'p Node>),
}

impl<Node, Parent> ProjRef<Node, Parent> {
    fn apply<'p>(&self, parent: &'p Parent) -> &'p Node {
        match self {
            Self::Bare(f) => f(parent),
            Self::Dyn(f) => f(parent),
        }
    }
}
```

`PathMut` before:

```rust
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
}
```

after:

```rust
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
    shared: ProjRef<Node, Parent>,
}
```

## change 2: the constructors take both

`from_fn` before:

```rust
    /// Builds a path from a parent and a non-capturing projection.
    #[must_use]
    pub const fn from_fn(parent: Parent, projection: fn(&mut Parent) -> &mut Node) -> Self {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
        }
    }
```

after:

```rust
    /// Builds a path from a parent and its two projections: one to read the node, one to write it.
    ///
    /// They must address the same node. Nothing checks that, and a pair that disagrees is a path
    /// whose reads and writes land in different places.
    #[must_use]
    pub const fn from_fn(
        parent: Parent,
        projection: fn(&mut Parent) -> &mut Node,
        shared: fn(&Parent) -> &Node,
    ) -> Self {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
            shared: ProjRef::Bare(shared),
        }
    }
```

`from_box` takes the boxed pair the same way.

## change 3: reading a path

```rust
    /// Returns a shared reference to the focused node, re-derived from the parent.
    ///
    /// Takes `&self`, so it composes with [`parent`](Self::parent): a reader holds both at once.
    #[must_use]
    pub fn get(&self) -> &Node {
        self.shared.apply(&self.parent)
    }
```

`get_mut` is unchanged.

The `compile_fail` doctests on `PathMut` that demonstrate the unique borrow stay as they are: they are about `get_mut`, which still borrows the whole path, and about use after `into_parent`. A new doctest shows what `get` buys:

```rust
/// Reading composes with walking up, because both are shared:
///
/// ```
/// use laserbeam::PathMut;
/// struct Album { title: String }
/// let mut album = Album { title: "Kid A".to_owned() };
/// let path: PathMut<String, &mut Album> =
///     PathMut::from_fn(&mut album, |a| &mut a.title, |a| &a.title);
/// let title = path.get();
/// let parent = path.parent();
/// assert_eq!(title, &parent.title);
/// ```
```

## change 4: the derive writes the pair

`crates/bind_macro/src/lib.rs`, `Edge`, which emits every path construction. Wherever it writes `|p| &mut p.#field` it writes `|p| &p.#field` beside it, and wherever it writes a boxed projection for a routed or boxed child it writes the shared twin. The enum arms that project into a variant do the same: `|p| match p { Variant(v) => v, .. }` in both flavours.

Every call site in the workspace is generated, so nothing outside the derive changes.

## change 5: a trigger closure takes the path

`crates/bind_macro/src/lib.rs`, `trigger_expr`'s callers. A place node hands the closure `&*path` for the root and `&*path` for a deeper node too, since a shared reference to the path is now enough to read through:

before:

```rust
    let node_ref = if root {
        quote!(&*path)
    } else {
        quote!(&*path.get_mut())
    };
```

after:

```rust
    // A shared reference to what dispatch is holding: the root's path IS `&mut Self`, and a
    // deeper node's is a `PathMut`, which now reads through `get`.
    let node_ref = quote!(&*path);
```

A binding on a deeper node then reads its node through `get`, and its parent when it wants one:

```rust
#[bind(|nav| ArmedTimer::from_guard(nav.get().timeout()) => to_home)]
```

The root's closure is unchanged, since its path dereferences straight to the node.

`crates/bind/tests/common/mod.rs`'s deeper closure trigger changes with it, from `|child| WaitingFor(child.wants)` to `|child| WaitingFor(child.get().wants)`, and gains a case reading `parent()` from a trigger, which is the thing this makes possible.

`refactors/past/trigger-closures.md` describes what a closure receives; it gains the correction.
