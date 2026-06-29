//! Mutable resolved paths into a single-owner tree.
//!
//! This is the mutable counterpart to isograph's `resolve_position`. From a
//! `&mut Root` you resolve a typed [`Path`] to the single active leaf, mutate
//! that leaf through [`Path::get_mut`], and walk back up with
//! [`Path::into_parent`], holding exactly one live `&mut` at a time. There is no
//! `Rc`, no `RefCell`, and no `unsafe`.
//!
//! The [`Resolve`] trait (one per node, like isograph's `ResolvePosition`) is
//! what `#[derive(Rayban)]` implements; the running example lives in the
//! `freddie` workspace's design notes.
//!
//! # Example
//!
//! ```
//! use rayban::Path;
//!
//! enum Tree {
//!     Leaf(u32),
//! }
//!
//! let mut tree = Tree::Leaf(1);
//! // A path that re-derives the `u32` inside `Tree::Leaf` from a `&mut Tree`.
//! let mut path: Path<u32, &mut Tree> = Path::from_fn(&mut tree, |t| {
//!     let Tree::Leaf(n) = t;
//!     n
//! });
//! *path.get_mut() += 41;
//! let Tree::Leaf(n) = path.into_parent();
//! assert_eq!(*n, 42);
//! ```

/// The projection a [`Path`] uses to re-derive its focused node from the parent.
///
/// `Bare` is a function pointer (what the derive emits, since its match and
/// field projections capture nothing). `Dyn` is a boxed closure, for a
/// hand-written projection that closes over data the derive cannot see, such as
/// an externally supplied index.
enum Proj<Node, Parent> {
    Bare(fn(&mut Parent) -> &mut Node),
    Dyn(Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>),
}

impl<Node, Parent> Proj<Node, Parent> {
    fn apply<'p>(&self, parent: &'p mut Parent) -> &'p mut Node {
        match self {
            Self::Bare(f) => f(parent),
            Self::Dyn(f) => f(parent),
        }
    }
}

/// A typed, mutable path to a `Node`: its owned `Parent` plus the projection
/// that re-derives the `Node` from that parent.
///
/// The `Parent` is private, so the only way up is [`into_parent`](Path::into_parent),
/// which consumes the path. That, together with [`get_mut`](Path::get_mut)
/// borrowing the whole path, keeps a stale or aliasing reference from compiling.
///
/// You cannot hold the leaf and walk up at the same time. `get_mut` borrows the
/// whole path, so moving up while the leaf is still borrowed does not compile:
///
/// ```compile_fail
/// use rayban::Path;
/// let mut root = 0_u32;
/// let mut path: Path<u32, &mut u32> = Path::from_fn(&mut root, |r| &mut **r);
/// let leaf = path.get_mut();
/// let parent = path.into_parent(); // moves `path` while `leaf` still borrows it
/// let _ = (leaf, parent);
/// ```
///
/// A path is dead once you walk up from it, so use after `into_parent` does not
/// compile either:
///
/// ```compile_fail
/// use rayban::Path;
/// let mut root = 0_u32;
/// let mut path: Path<u32, &mut u32> = Path::from_fn(&mut root, |r| &mut **r);
/// let _parent = path.into_parent();
/// let _leaf = path.get_mut(); // `path` has already been moved
/// ```
///
/// The parent field is private; it is reachable only through the methods:
///
/// ```compile_fail
/// use rayban::Path;
/// let mut root = 0_u32;
/// let path: Path<u32, &mut u32> = Path::from_fn(&mut root, |r| &mut **r);
/// let _ = path.parent; // private field
/// ```
pub struct Path<Node, Parent> {
    parent: Parent,
    projection: Proj<Node, Parent>,
}

impl<Node, Parent> Path<Node, Parent> {
    /// Builds a path from a parent and a non-capturing projection.
    #[must_use]
    pub const fn from_fn(parent: Parent, projection: fn(&mut Parent) -> &mut Node) -> Self {
        Self {
            parent,
            projection: Proj::Bare(projection),
        }
    }

    /// Builds a path from a parent and a boxed, possibly capturing, projection.
    #[must_use]
    pub fn from_box(
        parent: Parent,
        projection: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>,
    ) -> Self {
        Self {
            parent,
            projection: Proj::Dyn(projection),
        }
    }

    /// Returns a mutable reference to the focused node, re-derived from the parent.
    #[must_use]
    pub fn get_mut(&mut self) -> &mut Node {
        self.projection.apply(&mut self.parent)
    }

    /// Returns a shared reference to the parent path, without consuming this one.
    #[must_use]
    pub const fn parent(&self) -> &Parent {
        &self.parent
    }

    /// Consumes the path and returns the parent, moving one level up the tree.
    #[must_use]
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}

/// Resolves the active leaf of a node, given that node's [`Path`]. Implemented
/// at every layer of the tree, the way isograph's `ResolvePosition` is.
///
/// The root's `Path<'a>` is `&'a mut Self`; every other node's is a
/// [`Path`] whose parent is the node's declared parent type. `resolve` takes the
/// path by value rather than `&mut self`, which is what lets the `&mut` move down
/// the tree without two live borrows existing at once.
pub trait Resolve {
    /// This node's path type. The root's is `&'a mut Self`.
    type Path<'a>
    where
        Self: 'a;
    /// The shared enum of every leaf the tree can resolve to.
    type Resolved<'a>
    where
        Self: 'a;
    /// Resolves the active leaf from this node's path.
    fn resolve<'a>(path: Self::Path<'a>) -> Self::Resolved<'a>
    where
        Self: 'a;
}

#[cfg(test)]
mod tests {
    use super::Path;

    struct Outer {
        inner: Inner,
    }
    struct Inner {
        value: u32,
    }

    #[test]
    fn from_fn_get_mut_into_parent() {
        let mut outer = Outer {
            inner: Inner { value: 1 },
        };
        let mut path: Path<Inner, &mut Outer> = Path::from_fn(&mut outer, |o| &mut o.inner);
        path.get_mut().value = 42;
        let recovered = path.into_parent();
        assert_eq!(recovered.inner.value, 42);
    }

    #[test]
    fn parent_reads_without_consuming() {
        let mut outer = Outer {
            inner: Inner { value: 7 },
        };
        let path: Path<Inner, &mut Outer> = Path::from_fn(&mut outer, |o| &mut o.inner);
        assert_eq!(path.parent().inner.value, 7);
        // Still usable afterwards because `parent` only borrows.
        assert_eq!(path.parent().inner.value, 7);
    }

    #[test]
    fn from_box_can_capture() {
        let mut items = vec![10_u32, 20, 30];
        let index = 1_usize;
        {
            let mut path: Path<u32, &mut Vec<u32>> = Path::from_box(
                &mut items,
                Box::new(move |v: &mut &mut Vec<u32>| &mut v[index]),
            );
            *path.get_mut() += 5;
        }
        assert_eq!(items[1], 25);
    }
}
