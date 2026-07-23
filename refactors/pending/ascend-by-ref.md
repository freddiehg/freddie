# ascend by shared reference

`laserbeam::Ascend` has one method, `ascend(self) -> Target`, which consumes the path and walks up to an ancestor by value. That is what a handler that mutates the root needs, and it is the only reach available. A handler that wants to READ an ancestor and keep using its own node has no way to: ascending consumes the whole chain, so the leaf is gone.

The by-reference read already exists one level at a time. `PathMut::parent(&self) -> &Parent` and `PathMut::get(&self) -> &Node` walk up and read through a shared borrow, and `ProjRef` exists precisely so a path can be read without being held uniquely. What is missing is the generic-depth version: chaining `parent().parent()...` to a named ancestor, the shared-borrow mirror of `ascend`.

We add it under the name `ascend`, and rename the consuming walk to `ascend_mut`, mirroring `get`/`get_mut`.

```rust
pub trait Ascend<Target> {
    fn ascend(&self) -> &Target;   // borrows, leaves the path alive
    fn ascend_mut(self) -> Target; // consumes, walks up by value
}
```

`ascend` takes `&self` and returns `&Target`. For a `PathMut` ancestor, `Target` is that ancestor's path type and the caller reads through it with `.get()`. For the root, whose path type is `MercuryPath<'a> = &'a mut Mercury`, `&Target` is `&&mut Mercury`.

The `Ascend<Target>` bound on every handler is unchanged; only the method name a consuming caller writes changes, from `ascend()` to `ascend_mut()`.

## Change 1: rename the consuming walk to `ascend_mut`

Prefactor. No new behavior; it frees the name `ascend` for change 2. Atomic: the laserbeam rename and every mercury callsite move in one commit, because the old name disappears.

### `crates/laserbeam/src/lib.rs`

The trait, before:

```rust
pub trait Ascend<Target> {
    fn ascend(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> Ascend<T> for T {
    fn ascend(self) -> T {
        self
    }
}
```

After:

```rust
pub trait Ascend<Target> {
    fn ascend_mut(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> Ascend<T> for T {
    fn ascend_mut(self) -> T {
        self
    }
}
```

The sugar on `PathMut`, before:

```rust
    #[must_use]
    pub fn ascend_to<Target>(self) -> Target
    where
        Self: Ascend<Target>,
    {
        Ascend::ascend(self)
    }
```

After:

```rust
    #[must_use]
    pub fn ascend_to_mut<Target>(self) -> Target
    where
        Self: Ascend<Target>,
    {
        Ascend::ascend_mut(self)
    }
```

The per-depth macro, before:

```rust
macro_rules! ascend_impls {
    ([$($acc:ident),*]) => {};
    ([$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<T, $($acc,)* $head> Ascend<T> for ascend_nest!(T $(, $acc)*, $head) {
            fn ascend(self) -> T {
                ascend_up!(self $(, $acc)*, $head)
            }
        }
        ascend_impls!([$($acc,)* $head] $(, $rest)*);
    };
}
```

After (only the method name changes):

```rust
macro_rules! ascend_impls {
    ([$($acc:ident),*]) => {};
    ([$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<T, $($acc,)* $head> Ascend<T> for ascend_nest!(T $(, $acc)*, $head) {
            fn ascend_mut(self) -> T {
                ascend_up!(self $(, $acc)*, $head)
            }
        }
        ascend_impls!([$($acc,)* $head] $(, $rest)*);
    };
}
```

`ascend_tests` only names the `Ascend<Target>` bound and never calls the method, so it compiles unchanged.

### mercury callsites

Every `.ascend()` becomes `.ascend_mut()`. The generic bounds `P: Ascend<MercuryPath<'a>>` are untouched. The full set:

- `crates/mercury/src/handlers/mod.rs:55` — `and_go_home_from(path.ascend(), effects)` becomes `and_go_home_from(path.ascend_mut(), effects)`
- `crates/mercury/src/handlers/home.rs:25,36,49,58,72,83` — each `node.parent.ascend()` becomes `node.parent.ascend_mut()`
- `crates/mercury/src/handlers/quit.rs:22` — `node.parent.ascend().typing_state...` becomes `node.parent.ascend_mut().typing_state...`
- `crates/mercury/src/handlers/overlay.rs:18` — `node.parent.ascend().toggle_overlay()` becomes `node.parent.ascend_mut().toggle_overlay()`
- `crates/mercury/src/handlers/nav.rs:22` — `let root: MercuryPath<'_> = path.ascend();` becomes `path.ascend_mut();`
- `crates/mercury/src/handlers/resize.rs:63,77` — `path.ascend()` / `node.parent.ascend()` become `..ascend_mut()`
- `crates/mercury/src/handlers/app.rs:58` — `let root: MercuryPath<'_> = path.ascend();` becomes `path.ascend_mut();`

## Change 2: add the by-reference `ascend`

Purely additive to laserbeam. Ships alone.

### `crates/laserbeam/src/lib.rs`

The trait doc and definition, after:

```rust
/// Walk up a path to an ancestor.
///
/// [`ascend`](Self::ascend) borrows: it takes `&self` and returns `&Target`, so a
/// handler can read an ancestor and keep using its own node. [`ascend_mut`](Self::ascend_mut)
/// consumes the path and returns the ancestor by value, which is how a handler that
/// mutates the root gets there.
///
/// Implemented for every path and for each of its ancestors, to twelve levels, so
/// a handler can be generic over "any path beneath this node" rather than naming
/// one. Use [`PathMut::ascend_to`] / [`PathMut::ascend_to_mut`] to name the target,
/// or let it be inferred.
///
/// ```ignore
/// fn read<'a, P: Ascend<LayerPath<'a>>>(path: &P) {
///     let layer: &LayerPath = path.ascend();
/// }
/// fn take<'a, P: Ascend<LayerPath<'a>>>(path: P) {
///     let layer: LayerPath = path.ascend_mut();
/// }
/// ```
///
/// The impls match on the shape of the path rather than on which node it is, so
/// no node is named and adding one needs no new impl: `NavLayerPath` is just an
/// alias for `PathMut<NavLayer, LayerPath<'a>>`, which is the depth-one shape.
///
/// There is one impl per depth, and they cannot overlap. For a single `Self` each
/// gives a different `Target`, and unifying two of them would need a type that
/// contains itself, which the occurs check rejects. That is why this needs no
/// phantom index to disambiguate, the way `frunk`'s `Here`/`There` does, and why
/// no index leaks into the bounds of a handler that uses it.
///
/// Only for trees where every node has one parent. A node with several declares
/// its parent as a route enum rather than a `PathMut`, so the shapes stop matching,
/// and the ascent would not be unique anyway.
pub trait Ascend<Target> {
    fn ascend(&self) -> &Target;
    fn ascend_mut(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> Ascend<T> for T {
    fn ascend(&self) -> &T {
        self
    }
    fn ascend_mut(self) -> T {
        self
    }
}
```

The sugar on `PathMut`, after (both forms present):

```rust
    /// Walk up to `Target` by shared reference, naming it rather than leaving it to
    /// inference. See [`ascend_to_mut`](Self::ascend_to_mut) for the consuming form.
    #[must_use]
    pub fn ascend_to<Target>(&self) -> &Target
    where
        Self: Ascend<Target>,
    {
        Ascend::ascend(self)
    }

    /// Walk up to `Target`, consuming the path, naming it rather than leaving it to
    /// inference.
    #[must_use]
    pub fn ascend_to_mut<Target>(self) -> Target
    where
        Self: Ascend<Target>,
    {
        Ascend::ascend_mut(self)
    }
```

A by-reference companion to `ascend_up!`, added next to it:

```rust
/// One `parent()` per type parameter, the shared-borrow mirror of `ascend_up!`.
macro_rules! ascend_up_ref {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        ascend_up_ref!($e.parent() $(, $rest)*)
    };
}
```

The per-depth macro gains the `ascend` method beside `ascend_mut`:

```rust
macro_rules! ascend_impls {
    ([$($acc:ident),*]) => {};
    ([$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<T, $($acc,)* $head> Ascend<T> for ascend_nest!(T $(, $acc)*, $head) {
            fn ascend(&self) -> &T {
                ascend_up_ref!(self $(, $acc)*, $head)
            }
            fn ascend_mut(self) -> T {
                ascend_up!(self $(, $acc)*, $head)
            }
        }
        ascend_impls!([$($acc,)* $head] $(, $rest)*);
    };
}
```

Each `.parent()` returns `&Parent`, and the next call lands on that `&PathMut` by autoref, so depth `d` produces `self.parent()` chained `d` times, of type `&T`.

### test

Added to the value-level `tests` module (the one that builds real `PathMut`s over `Sheer`/`Attack`), alongside `from_box_can_capture`:

```rust
    #[test]
    fn ascend_reads_an_ancestor_by_shared_ref() {
        type Outer<'a> = PathMut<Attack, &'a mut Sheer>;
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let outer: Outer = PathMut::from_fn(&mut album, |a| &mut a.heart, |a| &a.heart);
        let mut inner: PathMut<u32, Outer> =
            PathMut::from_fn(outer, |p| &mut p.get_mut().length, |p| &p.get().length);

        // Read the parent (Attack) by shared ref, without consuming the path.
        let attack: &Outer = inner.ascend_to::<Outer>();
        assert_eq!(attack.get().length, 7);

        // The leaf is still usable afterwards.
        *inner.get_mut() += 1;
        assert_eq!(*inner.get(), 8);
    }
```
