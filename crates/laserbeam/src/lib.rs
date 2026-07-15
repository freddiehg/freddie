//! A mutable typed cursor into a single-owner tree.
//!
//! A [`PathMut`] holds a `&mut` at some node and a projection down to a child. You read or write
//! the child through [`PathMut::get_mut`], and walk back up with [`PathMut::into_parent`], holding
//! exactly one live `&mut` at a time.
//!
//! ```
//! use laserbeam::PathMut;
//!
//! struct Album { title: String }
//! let mut album = Album { title: "A Night at the Opera".to_string() };
//!
//! let mut path: PathMut<String, &mut Album> = PathMut::from_fn(&mut album, |a| &mut a.title);
//! path.get_mut().push_str(" (Remastered)");
//! drop(path);
//!
//! assert_eq!(album.title, "A Night at the Opera (Remastered)");
//! ```


/// The projection a [`PathMut`] uses to re-derive its focused node from the parent.
///
/// `Bare` is a function pointer (what the derive emits, since its match and field projections capture nothing). `Dyn` is a boxed closure, for a hand-written projection that closes over data the derive cannot see, such as an externally supplied index.
enum ProjMut<Node, Parent> {
    Bare(fn(&mut Parent) -> &mut Node),
    Dyn(Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>),
}

impl<Node, Parent> ProjMut<Node, Parent> {
    fn apply<'p>(&self, parent: &'p mut Parent) -> &'p mut Node {
        match self {
            Self::Bare(f) => f(parent),
            Self::Dyn(f) => f(parent),
        }
    }
}

/// A typed, mutable path to a `Node`: its owned `Parent` plus the projection that re-derives the `Node` from that parent.
///
/// The `Parent` is private, so the only way up is [`into_parent`](PathMut::into_parent), which consumes the path. That, together with [`get_mut`](PathMut::get_mut) borrowing the whole path, keeps a stale or aliasing reference from compiling.
///
/// You cannot hold the leaf and walk up at the same time. `get_mut` borrows the whole path, so moving up while the leaf is still borrowed does not compile:
///
/// ```compile_fail
/// use laserbeam::PathMut;
/// let mut root = 0_u32;
/// let mut path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r);
/// let leaf = path.get_mut();
/// let parent = path.into_parent(); // moves `path` while `leaf` still borrows it
/// let _ = (leaf, parent);
/// ```
///
/// A path is dead once you walk up from it, so use after `into_parent` does not compile either:
///
/// ```compile_fail
/// use laserbeam::PathMut;
/// let mut root = 0_u32;
/// let mut path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r);
/// let _parent = path.into_parent();
/// let _leaf = path.get_mut(); // `path` has already been moved
/// ```
///
/// The parent field is private; it is reachable only through the methods:
///
/// ```compile_fail
/// use laserbeam::PathMut;
/// let mut root = 0_u32;
/// let path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r);
/// let _ = path.parent; // private field
/// ```
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
}

impl<Node, Parent> PathMut<Node, Parent> {
    /// Builds a path from a parent and a non-capturing projection.
    #[must_use]
    pub const fn from_fn(parent: Parent, projection: fn(&mut Parent) -> &mut Node) -> Self {
        Self {
            parent,
            projection: ProjMut::Bare(projection),
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
            projection: ProjMut::Dyn(projection),
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

#[cfg(test)]
mod tests {
    use super::PathMut;

    // "Sheer Heart Attack".
    struct Sheer {
        heart: Attack,
    }
    struct Attack {
        length: u32,
    }

    #[test]
    fn from_fn_get_mut_into_parent() {
        let mut album = Sheer {
            heart: Attack { length: 1 },
        };
        let mut path: PathMut<Attack, &mut Sheer> = PathMut::from_fn(&mut album, |a| &mut a.heart);
        path.get_mut().length = 42;
        let recovered = path.into_parent();
        assert_eq!(recovered.heart.length, 42);
    }

    #[test]
    fn parent_reads_without_consuming() {
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let path: PathMut<Attack, &mut Sheer> = PathMut::from_fn(&mut album, |a| &mut a.heart);
        assert_eq!(path.parent().heart.length, 7);
        // Still usable afterwards because `parent` only borrows.
        assert_eq!(path.parent().heart.length, 7);
    }

    #[test]
    fn from_box_can_capture() {
        // A setlist of track lengths.
        let mut setlist = vec![10_u32, 20, 30];
        let index = 1_usize;
        {
            let mut path: PathMut<u32, &mut Vec<u32>> = PathMut::from_box(
                &mut setlist,
                Box::new(move |v: &mut &mut Vec<u32>| &mut v[index]),
            );
            *path.get_mut() += 5;
        }
        assert_eq!(setlist[1], 25);
    }
}

/// Walk up a path to an ancestor, consuming it.
///
/// Implemented for every path and for each of its ancestors, to twelve levels, so
/// a handler can be generic over "any path beneath this node" rather than naming
/// one. Use [`PathMut::ascend_to`] to name the target, or let it be inferred.
///
/// ```ignore
/// fn to_home<'a, P: Ascend<LayerPath<'a>>>(path: P) {
///     let layer: LayerPath = path.ascend();
/// }
/// nav_path.ascend_to::<LayerPath>();
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
    /// Walk up to the target.
    fn ascend(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> Ascend<T> for T {
    fn ascend(self) -> T {
        self
    }
}

impl<Node, Parent> PathMut<Node, Parent> {
    /// Walk up to `Target`, naming it rather than leaving it to inference.
    ///
    /// Sugar, and the only way to name the target on the right. `Target` is a
    /// parameter of [`Ascend`] rather than of its method, so `path.ascend::<T>()`
    /// does not compile: the method takes no generic arguments. Without this you
    /// would name the target on the left, `let layer: LayerPath = path.ascend();`,
    /// or spell out `<HomeLayerPath as Ascend<LayerPath>>::ascend(path)`.
    #[must_use]
    pub fn ascend_to<Target>(self) -> Target
    where
        Self: Ascend<Target>,
    {
        Ascend::ascend(self)
    }
}

/// `PathMut<N0, PathMut<N1, .. T>>`, one level per type parameter.
macro_rules! ascend_nest {
    ($t:ident) => { $t };
    ($t:ident, $head:ident $(, $rest:ident)*) => {
        PathMut<$head, ascend_nest!($t $(, $rest)*)>
    };
}

/// One `into_parent()` per type parameter.
macro_rules! ascend_up {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        ascend_up!($e.into_parent() $(, $rest)*)
    };
}

/// One `Ascend` impl per depth, walking the list of type parameters.
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

ascend_impls!([], N0, N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11);

#[cfg(test)]
#[allow(dead_code)] // the impls are asserted at the type level, never called
mod ascend_tests {
    use crate::{Ascend, PathMut};

    struct Root;
    struct Target;
    type TargetPath<'a> = PathMut<Target, &'a mut Root>;

    struct N1;
    struct N2;
    struct N3;
    struct N4;
    struct N5;
    struct N6;
    struct N7;
    struct N8;
    struct N9;
    struct N10;
    struct N11;
    struct N12;

    type D1<'a> = PathMut<N1, TargetPath<'a>>;
    type D2<'a> = PathMut<N2, D1<'a>>;
    type D3<'a> = PathMut<N3, D2<'a>>;
    type D4<'a> = PathMut<N4, D3<'a>>;
    type D5<'a> = PathMut<N5, D4<'a>>;
    type D6<'a> = PathMut<N6, D5<'a>>;
    type D7<'a> = PathMut<N7, D6<'a>>;
    type D8<'a> = PathMut<N8, D7<'a>>;
    type D9<'a> = PathMut<N9, D8<'a>>;
    type D10<'a> = PathMut<N10, D9<'a>>;
    type D11<'a> = PathMut<N11, D10<'a>>;
    type D12<'a> = PathMut<N12, D11<'a>>;

    const fn ascends<'a, P: Ascend<TargetPath<'a>>>() {}

    /// Twelve levels, plus the identity. Fails to compile if the reach is short.
    #[test]
    fn ascends_from_every_depth_up_to_twelve() {
        ascends::<TargetPath<'_>>(); // depth 0, the identity impl
        ascends::<D1<'_>>();
        ascends::<D2<'_>>();
        ascends::<D6<'_>>();
        ascends::<D11<'_>>();
        ascends::<D12<'_>>();
    }

    /// A path ascends to every ancestor, not only the one it was written for.
    #[test]
    fn a_path_ascends_to_each_of_its_ancestors() {
        const fn to<T, P: Ascend<T>>() {}
        to::<D2<'_>, D12<'_>>();
        to::<D11<'_>, D12<'_>>();
        to::<TargetPath<'_>, D12<'_>>();
    }
}
