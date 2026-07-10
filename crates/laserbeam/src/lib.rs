//! Mutable resolved paths into a single-owner tree.
//!
//! From a `&mut Root` you resolve a typed [`Path`] to the single active leaf, mutate that leaf through [`Path::get_mut`], and walk back up with [`Path::into_parent`], holding exactly one live `&mut` at a time.
//!
//! The [`Resolve`] trait, one per node, is what `#[derive(Laserbeam)]` implements; the running example lives in the `freddie` workspace's design notes.
//!
//! # Example
//!
//! ```
//! use laserbeam::{Path, Laserbeam, Resolve};
//!
//! #[derive(Laserbeam)]
//! #[laserbeam_root(resolved = Resolved)]
//! enum MediaType {
//!     Album(Album),
//!     Single(Single),
//! }
//!
//! #[derive(Laserbeam)]
//! #[laserbeam(path = AlbumPath, resolved = Resolved)]
//! struct Album {
//!     title: String,
//! }
//!
//! #[derive(Laserbeam)]
//! #[laserbeam(path = SinglePath, resolved = Resolved)]
//! struct Single {
//!     title: String,
//! }
//!
//! type AlbumPath<'a> = Path<Album, &'a mut MediaType>;
//! type SinglePath<'a> = Path<Single, &'a mut MediaType>;
//!
//! enum Resolved<'a> {
//!     Album(AlbumPath<'a>),
//!     Single(SinglePath<'a>),
//! }
//!
//! let mut media = MediaType::Single(Single { title: "Bohemian Rhapsody".to_string() });
//!
//! // Resolve to the active leaf, mutate it.
//! match <MediaType as Resolve>::resolve(&mut media) {
//!     Resolved::Single(mut path) => path.get_mut().title.push_str(" (Remastered)"),
//!     Resolved::Album(_) => unreachable!("built a single"),
//! }
//!
//! let MediaType::Single(s) = &media else { unreachable!() };
//! assert_eq!(s.title, "Bohemian Rhapsody (Remastered)");
//! ```

pub use laserbeam_macro::Laserbeam;

/// The projection a [`Path`] uses to re-derive its focused node from the parent.
///
/// `Bare` is a function pointer (what the derive emits, since its match and field projections capture nothing). `Dyn` is a boxed closure, for a hand-written projection that closes over data the derive cannot see, such as an externally supplied index.
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

/// A typed, mutable path to a `Node`: its owned `Parent` plus the projection that re-derives the `Node` from that parent.
///
/// The `Parent` is private, so the only way up is [`into_parent`](Path::into_parent), which consumes the path. That, together with [`get_mut`](Path::get_mut) borrowing the whole path, keeps a stale or aliasing reference from compiling.
///
/// You cannot hold the leaf and walk up at the same time. `get_mut` borrows the whole path, so moving up while the leaf is still borrowed does not compile:
///
/// ```compile_fail
/// use laserbeam::Path;
/// let mut root = 0_u32;
/// let mut path: Path<u32, &mut u32> = Path::from_fn(&mut root, |r| &mut **r);
/// let leaf = path.get_mut();
/// let parent = path.into_parent(); // moves `path` while `leaf` still borrows it
/// let _ = (leaf, parent);
/// ```
///
/// A path is dead once you walk up from it, so use after `into_parent` does not compile either:
///
/// ```compile_fail
/// use laserbeam::Path;
/// let mut root = 0_u32;
/// let mut path: Path<u32, &mut u32> = Path::from_fn(&mut root, |r| &mut **r);
/// let _parent = path.into_parent();
/// let _leaf = path.get_mut(); // `path` has already been moved
/// ```
///
/// The parent field is private; it is reachable only through the methods:
///
/// ```compile_fail
/// use laserbeam::Path;
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

/// Resolves the active leaf of a node, given that node's [`Path`]. Implemented at every layer of the tree.
///
/// The root's `Path<'a>` is `&'a mut Self`; every other node's is a [`Path`] whose parent is the node's declared parent type. `resolve` takes the path by value rather than `&mut self`, which is what lets the `&mut` move down the tree without two live borrows existing at once.
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
        let mut path: Path<Attack, &mut Sheer> = Path::from_fn(&mut album, |a| &mut a.heart);
        path.get_mut().length = 42;
        let recovered = path.into_parent();
        assert_eq!(recovered.heart.length, 42);
    }

    #[test]
    fn parent_reads_without_consuming() {
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let path: Path<Attack, &mut Sheer> = Path::from_fn(&mut album, |a| &mut a.heart);
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
            let mut path: Path<u32, &mut Vec<u32>> = Path::from_box(
                &mut setlist,
                Box::new(move |v: &mut &mut Vec<u32>| &mut v[index]),
            );
            *path.get_mut() += 5;
        }
        assert_eq!(setlist[1], 25);
    }
}

/// Nest `Path` one level per type parameter, ending at the target path.
#[doc(hidden)]
#[macro_export]
macro_rules! __ascend_nest {
    ($lt:lifetime, $target:ident) => { $target<$lt> };
    ($lt:lifetime, $target:ident, $head:ident $(, $rest:ident)*) => {
        $crate::Path<$head, $crate::__ascend_nest!($lt, $target $(, $rest)*)>
    };
}

/// One `into_parent()` per type parameter.
#[doc(hidden)]
#[macro_export]
macro_rules! __ascend_up {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        $crate::__ascend_up!($e.into_parent() $(, $rest)*)
    };
}

/// One impl per depth, walking the list of type parameters.
#[doc(hidden)]
#[macro_export]
macro_rules! __ascend_impls {
    ($lt:lifetime, $target:ident, $trait:ident, [$($acc:ident),*]) => {};
    ($lt:lifetime, $target:ident, $trait:ident, [$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<$lt, $($acc,)* $head> $trait<$lt>
            for $crate::__ascend_nest!($lt, $target $(, $acc)*, $head)
        {
            fn ascend(self) -> $target<$lt> {
                $crate::__ascend_up!(self $(, $acc)*, $head)
            }
        }
        $crate::__ascend_impls!($lt, $target, $trait, [$($acc,)* $head] $(, $rest)*);
    };
}

/// Declare a trait for ascending to `$target`, and implement it for `$target`
/// itself and for every path beneath it, to a depth of twelve.
///
/// The impls match on the shape of the path rather than on which node it is, so
/// no node is named and adding one needs no new impl. `HomeLayerPath` is just an
/// alias for `Path<HomeLayer, LayerPath<'a>>`, which is the depth-one shape.
///
/// They cannot overlap: unifying two of them would require a type that contains
/// itself, which the occurs check rejects. That is why this needs no phantom
/// index to disambiguate, the way `frunk`'s `Here`/`There` does.
///
/// ```ignore
/// laserbeam::impl_ascend!(LayerPath, ToLayerPath);
///
/// fn to_home<'a, P: ToLayerPath<'a>>(path: P) { /* path.ascend() is a LayerPath */ }
/// ```
///
/// Only for trees where every node has one parent. A node with several declares
/// its parent as a route enum rather than a `Path`, so the shapes stop matching,
/// and the ascent would not be unique anyway.
#[macro_export]
macro_rules! impl_ascend {
    ($target:ident, $trait:ident) => {
        /// A path that can ascend to the target path, consuming itself.
        pub trait $trait<'a> {
            /// Walk up to the target path.
            fn ascend(self) -> $target<'a>;
        }

        impl<'a> $trait<'a> for $target<'a> {
            fn ascend(self) -> Self {
                self
            }
        }

        $crate::__ascend_impls!(
            'a, $target, $trait, [],
            N0, N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11
        );
    };
}

#[cfg(test)]
#[allow(dead_code)] // the impls are asserted at the type level, never called
mod ascend_tests {
    use crate::Path;

    pub struct Root;
    pub struct Target;
    pub type TargetPath<'a> = Path<Target, &'a mut Root>;

    crate::impl_ascend!(TargetPath, ToTarget);

    pub struct N1;
    pub struct N2;
    pub struct N3;
    pub struct N4;
    pub struct N5;
    pub struct N6;
    pub struct N7;
    pub struct N8;
    pub struct N9;
    pub struct N10;
    pub struct N11;
    pub struct N12;

    pub type D1<'a> = Path<N1, TargetPath<'a>>;
    pub type D2<'a> = Path<N2, D1<'a>>;
    pub type D3<'a> = Path<N3, D2<'a>>;
    pub type D4<'a> = Path<N4, D3<'a>>;
    pub type D5<'a> = Path<N5, D4<'a>>;
    pub type D6<'a> = Path<N6, D5<'a>>;
    pub type D7<'a> = Path<N7, D6<'a>>;
    pub type D8<'a> = Path<N8, D7<'a>>;
    pub type D9<'a> = Path<N9, D8<'a>>;
    pub type D10<'a> = Path<N10, D9<'a>>;
    pub type D11<'a> = Path<N11, D10<'a>>;
    pub type D12<'a> = Path<N12, D11<'a>>;

    const fn assert_ascends<'a, P: ToTarget<'a>>() {}

    /// The macro claims twelve levels. This fails to compile if it does not.
    #[test]
    fn ascends_from_every_depth_up_to_twelve() {
        assert_ascends::<TargetPath<'_>>(); // depth 0, the identity impl
        assert_ascends::<D1<'_>>();
        assert_ascends::<D2<'_>>();
        assert_ascends::<D6<'_>>();
        assert_ascends::<D11<'_>>();
        assert_ascends::<D12<'_>>();
    }
}
