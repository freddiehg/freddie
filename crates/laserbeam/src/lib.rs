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
//! let mut path: PathMut<String, &mut Album> = PathMut::from_fn(&mut album, |a| &mut a.title, |a| &a.title);
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

/// The projection a [`PathMut`] uses to re-derive its focused node for READING.
///
/// Stored beside [`ProjMut`] rather than derived from it: applying that one needs `&mut Parent`,
/// which a shared borrow of the path cannot produce, so without this a path could only be read
/// uniquely.
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

/// A typed, mutable path to a `Node`: its owned `Parent` plus the projection that re-derives the `Node` from that parent.
///
/// The `Parent` is private, so the only way up is [`into_parent`](PathMut::into_parent), which consumes the path. That, together with [`get_mut`](PathMut::get_mut) borrowing the whole path, keeps a stale or aliasing reference from compiling.
///
/// You cannot hold the leaf and walk up at the same time. `get_mut` borrows the whole path, so moving up while the leaf is still borrowed does not compile:
///
/// ```compile_fail
/// use laserbeam::PathMut;
/// let mut root = 0_u32;
/// let mut path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r, |r| &**r);
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
/// let mut path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r, |r| &**r);
/// let _parent = path.into_parent();
/// let _leaf = path.get_mut(); // `path` has already been moved
/// ```
///
/// The parent field is private; it is reachable only through the methods:
///
/// ```compile_fail
/// use laserbeam::PathMut;
/// let mut root = 0_u32;
/// let path: PathMut<u32, &mut u32> = PathMut::from_fn(&mut root, |r| &mut **r, |r| &**r);
/// let _ = path.parent; // private field
/// ```
pub struct PathMut<Node, Parent> {
    parent: Parent,
    projection: ProjMut<Node, Parent>,
    shared: ProjRef<Node, Parent>,
}

impl<Node, Parent> PathMut<Node, Parent> {
    /// Builds a path from a parent and its two non-capturing projections: one to write the node,
    /// one to read it.
    ///
    /// The pair has to address the same node. Nothing checks that, and one that disagrees is a
    /// path whose reads and writes land in different places.
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

    /// Builds a path from a parent and boxed, possibly capturing, projections.
    ///
    /// The pair has to address the same node, as in [`from_fn`](Self::from_fn).
    #[must_use]
    pub fn from_box(
        parent: Parent,
        projection: Box<dyn for<'p> Fn(&'p mut Parent) -> &'p mut Node>,
        shared: Box<dyn for<'p> Fn(&'p Parent) -> &'p Node>,
    ) -> Self {
        Self {
            parent,
            projection: ProjMut::Dyn(projection),
            shared: ProjRef::Dyn(shared),
        }
    }

    /// Returns a shared reference to the focused node, re-derived from the parent.
    ///
    /// Takes `&self`, so it composes with [`parent`](Self::parent): a reader holds both at once,
    /// and a caller that only reads does not take the path uniquely.
    #[must_use]
    pub fn get(&self) -> &Node {
        self.shared.apply(&self.parent)
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
        let mut path: PathMut<Attack, &mut Sheer> =
            PathMut::from_fn(&mut album, |a| &mut a.heart, |a| &a.heart);
        path.get_mut().length = 42;
        let recovered = path.into_parent();
        assert_eq!(recovered.heart.length, 42);
    }

    #[test]
    fn parent_reads_without_consuming() {
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let path: PathMut<Attack, &mut Sheer> =
            PathMut::from_fn(&mut album, |a| &mut a.heart, |a| &a.heart);
        assert_eq!(path.parent().heart.length, 7);
        // Still usable afterwards because `parent` only borrows.
        assert_eq!(path.parent().heart.length, 7);
    }

    #[test]
    fn from_box_can_capture() {
        let mut setlist = vec![10_u32, 20, 30];
        let index = 1_usize;
        {
            let mut path: PathMut<u32, &mut Vec<u32>> = PathMut::from_box(
                &mut setlist,
                Box::new(move |v: &mut &mut Vec<u32>| &mut v[index]),
                Box::new(move |v: &&mut Vec<u32>| &v[index]),
            );
            assert_eq!(*path.get(), 20);
            *path.get_mut() += 5;
        }
        assert_eq!(setlist[1], 25);
    }

    #[test]
    fn ancestor_reads_by_shared_ref() {
        type Outer<'a> = PathMut<Attack, &'a mut Sheer>;
        let mut album = Sheer {
            heart: Attack { length: 7 },
        };
        let outer: Outer = PathMut::from_fn(&mut album, |a| &mut a.heart, |a| &a.heart);
        let mut inner: PathMut<u32, Outer> =
            PathMut::from_fn(outer, |p| &mut p.get_mut().length, |p| &p.get().length);

        // Read the parent (Attack) by shared ref, without consuming the path.
        let attack: &Outer = inner.ancestor::<Outer>();
        assert_eq!(attack.get().length, 7);

        *inner.get_mut() += 1;
        assert_eq!(*inner.get(), 8);
    }
}

/// Walk up a path to an ancestor by shared reference.
///
/// Takes `&self` and returns `&Target`, so a handler can read an ancestor and keep
/// using its own node. [`IntoAncestor`] is the consuming mirror, for a handler that
/// walks up to mutate.
///
/// Implemented for every path and for each of its ancestors, to twelve levels, so
/// a handler can be generic over "any path beneath this node" rather than naming
/// one. Use [`PathMut::ancestor`] to name the target, or let it be inferred.
///
/// ```ignore
/// fn read<'a, P: HasAncestor<LayerPath<'a>>>(path: &P) {
///     let layer: &LayerPath = path.ancestor();
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
/// and the walk would not be unique anyway.
pub trait HasAncestor<Target> {
    fn ancestor(&self) -> &Target;
}

/// Walk up a path to an ancestor, consuming it.
///
/// The consuming mirror of [`HasAncestor`]: takes `self` and returns `Target` by value,
/// which is how a handler that mutates the ancestor gets there. Shares [`HasAncestor`]'s
/// per-depth impl structure and overlap-freedom. Use [`PathMut::into_ancestor`] to
/// name the target, or let it be inferred.
///
/// [`HasAncestor`] is a supertrait: a consuming walk can always also borrow, so one
/// `IntoAncestor` bound gives a handler both reaches.
///
/// ```ignore
/// fn take<'a, P: IntoAncestor<LayerPath<'a>>>(path: P) {
///     let layer: LayerPath = path.into_ancestor();
/// }
/// ```
pub trait IntoAncestor<Target>: HasAncestor<Target> {
    fn into_ancestor(self) -> Target;
}

/// Every path is its own ancestor, at depth zero.
impl<T> HasAncestor<T> for T {
    fn ancestor(&self) -> &T {
        self
    }
}

impl<T> IntoAncestor<T> for T {
    fn into_ancestor(self) -> T {
        self
    }
}

impl<Node, Parent> PathMut<Node, Parent> {
    /// Walk up to `Target` by shared reference, naming it rather than leaving it to
    /// inference. See [`into_ancestor`](Self::into_ancestor) for the consuming form.
    #[must_use]
    pub fn ancestor<Target>(&self) -> &Target
    where
        Self: HasAncestor<Target>,
    {
        HasAncestor::ancestor(self)
    }

    /// Walk up to `Target`, consuming the path, naming it rather than leaving it to
    /// inference.
    ///
    /// Sugar, and the only way to name the target on the right. `Target` is a
    /// parameter of [`IntoAncestor`] rather than of its method, so
    /// `path.into_ancestor::<T>()` on the trait method alone does not compile: the
    /// method takes no generic arguments. The inherent method does take them, so
    /// `path.into_ancestor::<T>()` lands here. Without this you would name the
    /// target on the left, `let layer: LayerPath = path.into_ancestor();`, or write
    /// out `<HomeLayerPath as IntoAncestor<LayerPath>>::into_ancestor(path)`.
    #[must_use]
    pub fn into_ancestor<Target>(self) -> Target
    where
        Self: IntoAncestor<Target>,
    {
        IntoAncestor::into_ancestor(self)
    }
}

/// `PathMut<N0, PathMut<N1, .. T>>`, one level per type parameter.
///
/// The terminal is any type (`ty`), so a nest can end in a path alias rather than
/// only a bare identifier.
macro_rules! path_nest {
    ($t:ty) => { $t };
    ($t:ty, $head:ident $(, $rest:ident)*) => {
        PathMut<$head, path_nest!($t $(, $rest)*)>
    };
}

/// One `into_parent()` per type parameter.
macro_rules! into_parent_chain {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        into_parent_chain!($e.into_parent() $(, $rest)*)
    };
}

/// One `parent()` per type parameter, the shared-borrow mirror of `into_parent_chain!`.
macro_rules! parent_chain {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        parent_chain!($e.parent() $(, $rest)*)
    };
}

/// One `HasAncestor` and one `IntoAncestor` impl per depth, walking the list of type parameters.
macro_rules! ancestor_impls {
    ([$($acc:ident),*]) => {};
    ([$($acc:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<T, $($acc,)* $head> HasAncestor<T> for path_nest!(T $(, $acc)*, $head) {
            fn ancestor(&self) -> &T {
                parent_chain!(self $(, $acc)*, $head)
            }
        }
        impl<T, $($acc,)* $head> IntoAncestor<T> for path_nest!(T $(, $acc)*, $head) {
            fn into_ancestor(self) -> T {
                into_parent_chain!(self $(, $acc)*, $head)
            }
        }
        ancestor_impls!([$($acc,)* $head] $(, $rest)*);
    };
}

ancestor_impls!([], N0, N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11);

/// Where a leave stopped: at this path, or somewhere further up.
///
/// No derives: paths are neither `Debug` nor `PartialEq`; consumers
/// destructure.
pub enum Stop<H, U> {
    Here(H),
    Up(U),
}

/// A child of the root, unwrapped: the `Up` payload is the bare root path.
impl<'a, N, R> Stop<PathMut<N, &'a mut R>, &'a mut R> {
    /// The child's leave, as the state it leaves behind at the PARENT: stopping
    /// at the child leaves the parent standing one step up.
    #[must_use]
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<&'a mut R> {
        match self {
            Self::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Self::Up(root) => MaybeInvalidated::Invalidated(root.complete()),
        }
    }
}

/// A child of a non-root, unwrapped: the `Up` payload is the parent's own leave.
impl<N, N2, Q: Above> Stop<PathMut<N, PathMut<N2, Q>>, Completed<PathMut<N2, Q>>> {
    /// The child's leave, as the state it leaves behind at the PARENT.
    #[must_use]
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<PathMut<N2, Q>> {
        match self {
            Self::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Self::Up(rest) => MaybeInvalidated::Invalidated(rest),
        }
    }
}

/// What a completed leave hands upward once it has peeled past a child of
/// this path: the root path itself, or the completed leave from this path.
pub trait Above {
    type Up;
}

impl<'a, R> Above for &'a mut R {
    type Up = &'a mut R;
}

impl<N, P: Above> Above for PathMut<N, P> {
    type Up = Completed<Self>;
}

/// A node's path type.
///
/// `bind`'s derive implements it for every node that IS in the tree, from the node's
/// `#[node(parent_path = ..)]` or `#[node(root)]`.
pub trait HasPath {
    /// This node's path type. The three node kinds, in mercury's tree:
    ///
    /// - The root, `#[node(root)]`: `Mercury`'s is `&'a mut Mercury` — the bare exclusive
    ///   borrow, since there is nothing above to thread through.
    /// - A node reached through `#[child]`, `#[node(parent_path = ..)]`: `TypingLayer`'s
    ///   is `PathMut<TypingLayer, LayerPath<'a>>`, its declared `TypingLayerPath` alias — the
    ///   node over its parent's path, recursively, so the whole ancestry rides along.
    /// - A derived level (`ClaudeAiSite`) has no path and does not implement this trait: its
    ///   data is rebuilt on every dispatch and dies with it, and it ascends at the place
    ///   beneath it (`SiteLayerPath<'a>`), which is where a path exists.
    type Path<'a>
    where
        Self: 'a;
}

/// A path's stop layer: stopped here, or went above. A root path can only
/// stop at itself, so its layer is the bare path.
pub trait HasStop: Sized {
    type Stop;

    /// A completed leave, as the state it leaves behind at this path.
    ///
    /// A leave that stopped here left the path standing, so the path comes back
    /// out; one that went above destroyed it, and the completed leave is handed
    /// on for whoever is still holding the path type to forward.
    fn to_maybe_invalidated(completed: Completed<Self>) -> MaybeInvalidated<Self>;
}

impl<N, P: Above> HasStop for PathMut<N, P> {
    type Stop = Stop<Self, P::Up>;

    fn to_maybe_invalidated(completed: Completed<Self>) -> MaybeInvalidated<Self> {
        match completed.into_inner() {
            Stop::Here(path) => MaybeInvalidated::NotInvalidated(path),
            Stop::Up(rest) => MaybeInvalidated::Invalidated(Completed::up(rest)),
        }
    }
}

impl<'a, R> HasStop for &'a mut R {
    type Stop = &'a mut R;

    fn to_maybe_invalidated(completed: Completed<Self>) -> MaybeInvalidated<Self> {
        MaybeInvalidated::NotInvalidated(completed.into_inner())
    }
}

/// Have we destroyed the path we need?
///
/// What a node holds of its own path once the descent below it has run: the
/// child either left it standing, or completed a leave that peeled past it.
///
/// No derives, for the reason [`Stop`] has none: paths are neither `Debug` nor
/// `PartialEq`.
pub enum MaybeInvalidated<P: HasStop> {
    /// No: here it is.
    NotInvalidated(P),
    /// Yes: the completed leave, ready to forward. A [`Stop::Here`] inside it
    /// means the leave stopped at this path, so the path is recoverable.
    Invalidated(Completed<P>),
}

impl<P: HasStop + CompletesTo<P>> MaybeInvalidated<P> {
    /// The leave this state completes to: the path completing where it stands,
    /// or the leave that already went past it.
    #[must_use]
    pub fn complete(self) -> Completed<P> {
        match self {
            Self::NotInvalidated(path) => path.complete(),
            Self::Invalidated(completed) => completed,
        }
    }
}

impl<P: HasStop + CompletesTo<P>> MaybeInvalidated<P>
where
    Completed<P>: TryIntoAncestor<P>,
{
    /// Descend the next child subtree with this node's path, if the path can
    /// still be built: lent from a standing state, or recovered from a leave that
    /// stopped exactly here — in which case the node stays invalidated whatever
    /// the child does. A leave that went above skips the descent and forwards.
    #[must_use]
    pub fn descend(self, f: impl FnOnce(P) -> Self) -> Self {
        match self {
            Self::NotInvalidated(p) => f(p),
            Self::Invalidated(completed) => match completed.try_into_ancestor() {
                Ok(p) => Self::Invalidated(f(p).complete()),
                Err(completed) => Self::Invalidated(completed),
            },
        }
    }
}

impl<P: HasStop> MaybeInvalidated<P> {
    /// Walk this state to `Target` by shared reference, on either branch.
    #[must_use]
    pub fn ancestor<Target>(&self) -> &Target
    where
        Self: HasAncestor<Target>,
    {
        HasAncestor::ancestor(self)
    }

    /// Walk this state to `Target`, consuming it, on either branch.
    #[must_use]
    pub fn into_ancestor<Target>(self) -> Target
    where
        Self: IntoAncestor<Target>,
    {
        IntoAncestor::into_ancestor(self)
    }

    /// Walk this state to `Target` if it is still standing.
    ///
    /// # Errors
    ///
    /// The leave this state holds went above `Target`; the state comes back,
    /// ready to forward.
    pub fn try_into_ancestor<Target>(self) -> Result<Target, Self>
    where
        Self: TryIntoAncestor<Target>,
    {
        TryIntoAncestor::try_into_ancestor(self)
    }
}

/// The state after a descent holds the root on both branches: through the
/// standing path, or through the leave that replaced it.
///
/// A handler whose meaning is that the dispatch ends at the root stops matching
/// the state at all: it reaches the root either way, at every depth.
impl<'a, R, P> HasAncestor<&'a mut R> for MaybeInvalidated<P>
where
    P: HasStop + HasAncestor<&'a mut R>,
    Completed<P>: HasAncestor<&'a mut R>,
{
    fn ancestor(&self) -> &&'a mut R {
        match self {
            Self::NotInvalidated(path) => HasAncestor::ancestor(path),
            Self::Invalidated(completed) => HasAncestor::ancestor(completed),
        }
    }
}

impl<'a, R, P> IntoAncestor<&'a mut R> for MaybeInvalidated<P>
where
    P: HasStop + IntoAncestor<&'a mut R>,
    Completed<P>: IntoAncestor<&'a mut R>,
{
    fn into_ancestor(self) -> &'a mut R {
        match self {
            Self::NotInvalidated(path) => IntoAncestor::into_ancestor(path),
            Self::Invalidated(completed) => IntoAncestor::into_ancestor(completed),
        }
    }
}

/// Reach a chain ancestor that a completed leave may have destroyed.
///
/// `Ok` iff the leave stopped at or below the target, so the target is still
/// standing: here it is, consumed out of the leave. `Err` gives the value
/// back unchanged, because the caller still has to return a `Completed` and
/// must be able to forward the leave it could not use. To the root the answer
/// is always `Ok`; the total [`IntoAncestor`] says the same thing without the
/// `Result`.
pub trait TryIntoAncestor<Target>: Sized {
    /// # Errors
    ///
    /// The leave went above `Target`, which no longer exists; the value comes
    /// back so the caller can forward it.
    fn try_into_ancestor(self) -> Result<Target, Self>;
}

/// Distance zero: the leave reaches its own origin iff it stopped there.
impl<T: HasStop> TryIntoAncestor<T> for Completed<T> {
    fn try_into_ancestor(self) -> Result<T, Self> {
        match self.to_maybe_invalidated() {
            MaybeInvalidated::NotInvalidated(path) => Ok(path),
            MaybeInvalidated::Invalidated(completed) => Err(completed),
        }
    }
}

/// Distance one to the root: the root is always alive.
impl<'a, R, H> TryIntoAncestor<&'a mut R> for Completed<PathMut<H, &'a mut R>> {
    fn try_into_ancestor(self) -> Result<&'a mut R, Self> {
        match self.stop {
            Stop::Here(path) => Ok(path.into_parent()),
            Stop::Up(root) => Ok(root),
        }
    }
}

/// Distance one to a non-root ancestor: alive iff the leave stopped at or
/// below it.
impl<H, N2, Q: Above> TryIntoAncestor<PathMut<N2, Q>> for Completed<PathMut<H, PathMut<N2, Q>>> {
    fn try_into_ancestor(self) -> Result<PathMut<N2, Q>, Self> {
        match self.stop {
            Stop::Here(path) => Ok(path.into_parent()),
            Stop::Up(up) => up.try_into_ancestor().map_err(Self::up),
        }
    }
}

/// One impl per distance of two or more: the `Here` arm walks the standing
/// path up, and the `Up` arm hands the question to the parent's leave.
macro_rules! try_into_ancestor_impls {
    ($head:ident) => {};
    ($head:ident, $next:ident $(, $rest:ident)*) => {
        impl<T, $head, $next $(, $rest)*> TryIntoAncestor<T>
            for Completed<path_nest!(T, $head, $next $(, $rest)*)>
        where
            T: Above,
            Completed<path_nest!(T, $next $(, $rest)*)>: TryIntoAncestor<T>,
        {
            fn try_into_ancestor(self) -> Result<T, Self> {
                match self.stop {
                    Stop::Here(path) => Ok(path.into_ancestor()),
                    Stop::Up(up) => up.try_into_ancestor().map_err(Completed::up),
                }
            }
        }
        try_into_ancestor_impls!($next $(, $rest)*);
    };
}

try_into_ancestor_impls!(M1, M2, M3, M4, M5, M6, M7, M8, M9, M10, M11, M12);

/// On the state: a standing path reaches every chain ancestor; an invalidated
/// one asks its leave.
impl<P, T> TryIntoAncestor<T> for MaybeInvalidated<P>
where
    P: HasStop + IntoAncestor<T>,
    Completed<P>: TryIntoAncestor<T>,
{
    fn try_into_ancestor(self) -> Result<T, Self> {
        match self {
            Self::NotInvalidated(path) => Ok(path.into_ancestor()),
            Self::Invalidated(completed) => {
                TryIntoAncestor::try_into_ancestor(completed).map_err(Self::Invalidated)
            }
        }
    }
}

/// A completed leave from origin `P`: where the peeling stopped.
///
/// [`CompletesTo::complete`] and [`Completed::up`] construct one; `new` is private.
/// Consumers get [`into_inner`](Self::into_inner) to unwrap.
pub struct Completed<P: HasStop> {
    stop: P::Stop,
}

impl<P: HasStop> Completed<P> {
    const fn new(stop: P::Stop) -> Self {
        Self { stop }
    }

    #[must_use]
    pub fn into_inner(self) -> P::Stop {
        self.stop
    }

    /// This leave, as the state it leaves behind at `P`.
    #[must_use]
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<P> {
        P::to_maybe_invalidated(self)
    }

    /// Walk this leave to `Target` by shared reference, naming it rather than
    /// leaving it to inference.
    #[must_use]
    pub fn ancestor<Target>(&self) -> &Target
    where
        Self: HasAncestor<Target>,
    {
        HasAncestor::ancestor(self)
    }

    /// Walk this leave to `Target`, consuming it, naming the target.
    #[must_use]
    pub fn into_ancestor<Target>(self) -> Target
    where
        Self: IntoAncestor<Target>,
    {
        IntoAncestor::into_ancestor(self)
    }

    /// Walk this leave to `Target` if the leave left it standing.
    ///
    /// # Errors
    ///
    /// The leave went above `Target`; it comes back, ready to forward.
    pub fn try_into_ancestor<Target>(self) -> Result<Target, Self>
    where
        Self: TryIntoAncestor<Target>,
    {
        TryIntoAncestor::try_into_ancestor(self)
    }
}

/// A leave from the root holds the root.
impl<'a, R> HasAncestor<&'a mut R> for Completed<&'a mut R> {
    fn ancestor(&self) -> &&'a mut R {
        &self.stop
    }
}

impl<'a, R> IntoAncestor<&'a mut R> for Completed<&'a mut R> {
    fn into_ancestor(self) -> &'a mut R {
        self.stop
    }
}

/// A leave from a path holds the root through whichever arm it stopped in:
/// through the path it left standing, or through the leave above it.
///
/// The root is the only ancestor a leave holds on every inhabitant. `Completed`
/// erases where the leave stopped, which is what lets one handler return the same
/// type from every branch, so a shallower ancestor is simply absent from the
/// went-to-the-root one; reaching those is [`TryIntoAncestor`]'s question.
impl<'a, R, N, P> HasAncestor<&'a mut R> for Completed<PathMut<N, P>>
where
    P: Above,
    PathMut<N, P>: HasAncestor<&'a mut R>,
    P::Up: HasAncestor<&'a mut R>,
{
    fn ancestor(&self) -> &&'a mut R {
        match &self.stop {
            Stop::Here(path) => path.ancestor(),
            Stop::Up(rest) => HasAncestor::ancestor(rest),
        }
    }
}

impl<'a, R, N, P> IntoAncestor<&'a mut R> for Completed<PathMut<N, P>>
where
    P: Above,
    PathMut<N, P>: IntoAncestor<&'a mut R>,
    P::Up: IntoAncestor<&'a mut R>,
{
    fn into_ancestor(self) -> &'a mut R {
        match self.stop {
            Stop::Here(path) => path.into_ancestor(),
            Stop::Up(rest) => IntoAncestor::into_ancestor(rest),
        }
    }
}

/// The bare root path a leave from the root completes to, back as a leave.
///
/// A caller that has peeled past a child normalizes what it got, root path or
/// completed leave, through one `Into`.
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Self::new(root)
    }
}

impl<N, Par: Above> Completed<PathMut<N, Par>> {
    /// Rebuild "the leave went above this path" from the payload `into_inner`
    /// handed out: the inverse of unwrapping one `Up` level. A parent that
    /// inspects its child's leave, finds it went past it, and must still
    /// return its own `Completed` returns this.
    #[must_use]
    pub const fn up(above: Par::Up) -> Self {
        Self::new(Stop::Up(above))
    }
}

/// Complete a leave from origin `O` at this path: wrap the focus into
/// `Completed<O>` at its chain position.
///
/// `O` is a type parameter, not an associated type, because one focus
/// completes into every `Completed` whose chain contains it (a `LayerPath`
/// into `Completed<NavPath>`, `Completed<TypingPath>`, `Completed<LayerPath>`).
/// The call site's expected type pins `O`: the dispatch return type, or an
/// annotation.
///
/// Impls are indexed by peel distance, like `HasAncestor`: one impl for zero
/// peels at any depth, then per distance one impl for a focus still on the
/// chain and one for a focus at the root. Unifying two distances needs a type
/// that contains itself, which the occurs check rejects, so no phantom index
/// is needed. Off-chain completes have no impl and do not compile.
pub trait CompletesTo<O: HasStop> {
    fn complete(self) -> Completed<O>;
}

/// One `Completed::new(Stop::Up(..))` per peeled-past type parameter.
macro_rules! up_wrap {
    ($e:expr) => { $e };
    ($e:expr, $head:ident $(, $rest:ident)*) => {
        Completed::new(Stop::Up(up_wrap!($e $(, $rest)*)))
    };
}

/// Stopping at the origin: zero peels, every depth, one impl.
impl<N, P: Above> CompletesTo<Self> for PathMut<N, P> {
    fn complete(self) -> Completed<Self> {
        Completed::new(Stop::Here(self))
    }
}

/// Stopping at the root, for a leave that began there: the bare path.
impl<'a, R> CompletesTo<&'a mut R> for &'a mut R {
    fn complete(self) -> Completed<&'a mut R> {
        Completed::new(self)
    }
}

/// Two `CompletesTo` impls per peel distance: focus still a path, and focus at
/// the root. The origin in the trait parameter is the focus wrapped in one
/// `PathMut` per skipped layer.
macro_rules! complete_impls {
    ([$($done:ident),*]) => {};
    ([$($done:ident),*], $head:ident $(, $rest:ident)*) => {
        impl<$($done,)* $head, N, P: Above> CompletesTo<path_nest!(PathMut<N, P>, $($done,)* $head)>
            for PathMut<N, P>
        {
            fn complete(self) -> Completed<path_nest!(PathMut<N, P>, $($done,)* $head)> {
                up_wrap!(Completed::new(Stop::Here(self)), $($done,)* $head)
            }
        }

        impl<'a, R, $($done,)* $head> CompletesTo<path_nest!(&'a mut R, $($done,)* $head)>
            for &'a mut R
        {
            fn complete(self) -> Completed<path_nest!(&'a mut R, $($done,)* $head)> {
                up_wrap!(self, $($done,)* $head)
            }
        }

        complete_impls!([$($done,)* $head] $(, $rest)*);
    };
}

complete_impls!([], N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N11, N12);

#[cfg(test)]
mod ancestor_tests {
    use crate::{HasAncestor, IntoAncestor, PathMut};

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

    const fn reaches<'a, P: HasAncestor<TargetPath<'a>> + IntoAncestor<TargetPath<'a>>>() {}

    /// Twelve levels, plus the identity, for both traits. Fails to compile if either reach is short.
    #[test]
    fn reaches_from_every_depth_up_to_twelve() {
        reaches::<TargetPath<'_>>(); // depth 0, the identity impl
        reaches::<D1<'_>>();
        reaches::<D2<'_>>();
        reaches::<D6<'_>>();
        reaches::<D11<'_>>();
        reaches::<D12<'_>>();
    }

    /// A path reaches every ancestor, not only the one it was written for.
    #[test]
    fn a_path_reaches_each_of_its_ancestors() {
        const fn to<T, P: HasAncestor<T> + IntoAncestor<T>>() {}
        to::<D2<'_>, D12<'_>>();
        to::<D11<'_>, D12<'_>>();
        to::<TargetPath<'_>, D12<'_>>();
    }
}

#[cfg(test)]
mod complete_tests {
    use crate::{Completed, CompletesTo, HasStop, IntoAncestor, PathMut, Stop};

    struct App {
        hits: u32,
        layer: Layer,
    }
    struct Layer {
        nav: Nav,
    }
    struct Nav {
        hits: u32,
    }

    type AppPath<'a> = &'a mut App;
    type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
    type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;

    fn tree(nav_hits: u32, app_hits: u32) -> App {
        App {
            hits: app_hits,
            layer: Layer {
                nav: Nav { hits: nav_hits },
            },
        }
    }

    fn layer_path(app: &mut App) -> LayerPath<'_> {
        PathMut::from_fn(app, |a| &mut a.layer, |a| &a.layer)
    }

    fn nav_path(app: &mut App) -> NavPath<'_> {
        PathMut::from_fn(
            layer_path(app),
            |lp| &mut lp.get_mut().nav,
            |lp| &lp.get().nav,
        )
    }

    /// Pins the expanded Stop shapes for the three-level tree.
    #[allow(dead_code)]
    fn shapes<'a>(nav: Completed<NavPath<'a>>) {
        let stop: Stop<NavPath<'a>, Completed<LayerPath<'a>>> = nav.into_inner();
        if let Stop::Up(rest) = stop {
            let _: Stop<LayerPath<'a>, AppPath<'a>> = rest.into_inner();
        }
    }

    #[test]
    fn complete_at_nav() {
        let mut app = tree(7, 0);
        let out: Completed<NavPath<'_>> = nav_path(&mut app).complete();
        let Stop::Here(mut nav) = out.into_inner() else {
            panic!("expected Here");
        };
        assert_eq!(nav.get().hits, 7);
        nav.get_mut().hits = 8;
        drop(nav);
        assert_eq!(app.layer.nav.hits, 8);
    }

    #[test]
    fn one_peel() {
        let mut app = tree(7, 0);
        let out: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
        let Stop::Up(rest) = out.into_inner() else {
            panic!("expected Up");
        };
        let Stop::Here(layer) = rest.into_inner() else {
            panic!("expected Up(Here(layer))");
        };
        assert_eq!(layer.get().nav.hits, 7);
    }

    #[test]
    fn two_peels() {
        let mut app = tree(0, 0);
        {
            let out: Completed<NavPath<'_>> =
                nav_path(&mut app).into_parent().into_parent().complete();
            let Stop::Up(rest) = out.into_inner() else {
                panic!("expected Up");
            };
            let Stop::Up(root) = rest.into_inner() else {
                panic!("expected Up(Up(app))");
            };
            root.hits = 3;
        }
        assert_eq!(app.hits, 3);
    }

    #[test]
    fn layer_origin_bare_root() {
        let mut app = tree(0, 0);
        {
            let out: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
            let Stop::Up(root) = out.into_inner() else {
                panic!("expected Up(app)");
            };
            root.hits = 1;
        }
        assert_eq!(app.hits, 1);
    }

    #[test]
    fn root_completes_bare() {
        let mut app = tree(0, 0);
        {
            let out: Completed<AppPath<'_>> = (&mut app).complete();
            let root = out.into_inner();
            root.hits = 5;
        }
        assert_eq!(app.hits, 5);
    }

    fn stay<P: CompletesTo<P> + HasStop>(path: P) -> Completed<P> {
        path.complete()
    }

    fn to_root<'a, P>(path: P) -> Completed<P>
    where
        P: IntoAncestor<AppPath<'a>> + HasStop,
        AppPath<'a>: CompletesTo<P>,
    {
        path.into_ancestor().complete()
    }

    #[test]
    fn same_generic_handler_at_nav_and_root() {
        let mut app = tree(7, 0);

        let stay_nav: Completed<NavPath<'_>> = stay(nav_path(&mut app));
        let Stop::Here(nav) = stay_nav.into_inner() else {
            panic!("stay at nav is Here");
        };
        assert_eq!(nav.get().hits, 7);
        drop(nav);

        let stay_root: Completed<AppPath<'_>> = stay(&mut app);
        assert_eq!(stay_root.into_inner().hits, 0);

        let mut app = tree(0, 0);
        {
            let from_nav: Completed<NavPath<'_>> = to_root(nav_path(&mut app));
            let Stop::Up(rest) = from_nav.into_inner() else {
                panic!("to_root from nav peels");
            };
            let Stop::Up(root) = rest.into_inner() else {
                panic!("two peels to root");
            };
            assert_eq!(root.hits, 0);
        }

        let from_root: Completed<AppPath<'_>> = to_root(&mut app);
        assert_eq!(from_root.into_inner().hits, 0);
    }

    #[test]
    fn all_peel_depths_unify() {
        fn all_depths(nav: NavPath<'_>, branch: u8) -> Completed<NavPath<'_>> {
            match branch {
                0 => nav.complete(),
                1 => nav.into_parent().complete(),
                _ => nav.into_parent().into_parent().complete(),
            }
        }

        let mut app = tree(7, 0);
        {
            let here = all_depths(nav_path(&mut app), 0);
            let Stop::Here(nav) = here.into_inner() else {
                panic!("branch 0");
            };
            assert_eq!(nav.get().hits, 7);
        }
        {
            let one = all_depths(nav_path(&mut app), 1);
            let Stop::Up(rest) = one.into_inner() else {
                panic!("branch 1");
            };
            let Stop::Here(layer) = rest.into_inner() else {
                panic!("branch 1 Here(layer)");
            };
            assert_eq!(layer.get().nav.hits, 7);
        }
        {
            let two = all_depths(nav_path(&mut app), 2);
            let Stop::Up(rest) = two.into_inner() else {
                panic!("branch 2");
            };
            let Stop::Up(root) = rest.into_inner() else {
                panic!("branch 2 Up(app)");
            };
            assert_eq!(root.hits, 0);
        }
    }

    #[test]
    fn parent_returns_up_payload() {
        fn parent_returns_up_payload(child: Completed<NavPath<'_>>) -> Completed<LayerPath<'_>> {
            match child.into_inner() {
                Stop::Here(nav) => nav.into_parent().complete(),
                Stop::Up(rest) => rest,
            }
        }

        let mut app = tree(7, 0);
        let from_here = parent_returns_up_payload(nav_path(&mut app).complete());
        let Stop::Here(layer) = from_here.into_inner() else {
            panic!("Here arm peels to layer");
        };
        assert_eq!(layer.get().nav.hits, 7);

        let mut app = tree(7, 0);
        let from_up = parent_returns_up_payload(nav_path(&mut app).into_parent().complete());
        let Stop::Here(layer) = from_up.into_inner() else {
            panic!("Up arm is the layer Completed");
        };
        assert_eq!(layer.get().nav.hits, 7);

        let mut app = tree(0, 0);
        {
            let from_root = parent_returns_up_payload(
                nav_path(&mut app).into_parent().into_parent().complete(),
            );
            let Stop::Up(root) = from_root.into_inner() else {
                panic!("Up past layer is bare root");
            };
            root.hits = 9;
        }
        assert_eq!(app.hits, 9);
    }

    /// The inspecting parent form: two nested `into_inner` matches, with
    /// `Completed::up` rebuilding the gone-above arm.
    #[test]
    fn parent_inspects_and_rebuilds_with_up() {
        fn parent_inspect(child: Completed<NavPath<'_>>) -> Completed<LayerPath<'_>> {
            match child.into_inner() {
                Stop::Here(nav) => nav.into_parent().complete(),
                Stop::Up(rest) => match rest.into_inner() {
                    Stop::Here(layer) => layer.complete(),
                    Stop::Up(above) => Completed::up(above),
                },
            }
        }

        let mut app = tree(7, 0);
        let stopped_here = parent_inspect(nav_path(&mut app).into_parent().complete());
        let Stop::Here(layer) = stopped_here.into_inner() else {
            panic!("stopped at layer");
        };
        assert_eq!(layer.get().nav.hits, 7);

        let mut app = tree(0, 0);
        {
            let gone = parent_inspect(nav_path(&mut app).into_parent().into_parent().complete());
            let Stop::Up(root) = gone.into_inner() else {
                panic!("gone above layer");
            };
            root.hits = 4;
        }
        assert_eq!(app.hits, 4);
    }
}

#[cfg(test)]
mod maybe_invalidated_tests {
    use crate::{Completed, CompletesTo, MaybeInvalidated, PathMut, Stop};

    struct App {
        hits: u32,
        layer: Layer,
    }
    struct Layer {
        nav: Nav,
    }
    struct Nav {
        hits: u32,
    }

    type AppPath<'a> = &'a mut App;
    type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
    type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;

    const fn tree(nav_hits: u32, app_hits: u32) -> App {
        App {
            hits: app_hits,
            layer: Layer {
                nav: Nav { hits: nav_hits },
            },
        }
    }

    fn layer_path(app: &mut App) -> LayerPath<'_> {
        PathMut::from_fn(app, |a| &mut a.layer, |a| &a.layer)
    }

    fn nav_path(app: &mut App) -> NavPath<'_> {
        PathMut::from_fn(
            layer_path(app),
            |lp| &mut lp.get_mut().nav,
            |lp| &lp.get().nav,
        )
    }

    /// The root's own descent: a child that stopped at itself leaves the root
    /// standing, and one that went above hands the root back as a leave.
    #[test]
    fn a_root_reads_its_childs_leave_as_its_own_state() {
        let mut app = tree(7, 0);
        {
            let stayed: Completed<LayerPath<'_>> = layer_path(&mut app).complete();
            let MaybeInvalidated::NotInvalidated(root) = stayed.into_inner().to_maybe_invalidated()
            else {
                panic!("a child that stopped at itself leaves the root standing");
            };
            root.hits = 1;
        }
        assert_eq!(app.hits, 1);

        {
            let left: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
            let MaybeInvalidated::Invalidated(completed) = left.into_inner().to_maybe_invalidated()
            else {
                panic!("a child that went above destroyed the root's descent");
            };
            completed.into_inner().hits = 2;
        }
        assert_eq!(app.hits, 2);
    }

    /// The same, one level down: the `Up` payload is the parent's own leave
    /// rather than a bare root.
    #[test]
    fn a_layer_reads_its_childs_leave_as_its_own_state() {
        let mut app = tree(7, 0);
        {
            let stayed: Completed<NavPath<'_>> = nav_path(&mut app).complete();
            let MaybeInvalidated::NotInvalidated(layer) =
                stayed.into_inner().to_maybe_invalidated()
            else {
                panic!("nav stopped at itself, so the layer stands");
            };
            assert_eq!(layer.get().nav.hits, 7);
        }

        {
            let left: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
            let MaybeInvalidated::Invalidated(completed) = left.into_inner().to_maybe_invalidated()
            else {
                panic!("nav left, so the layer's descent is invalidated");
            };
            let Stop::Here(layer) = completed.into_inner() else {
                panic!("the leave stopped at the layer");
            };
            assert_eq!(layer.get().nav.hits, 7);
        }
    }

    /// The fold the generated code does after each scheduled item: a leave that
    /// stopped here re-establishes the path, one that went above stays a leave.
    #[test]
    fn a_returned_leave_folds_back_into_the_state() {
        let mut app = tree(7, 0);
        {
            let stayed: Completed<LayerPath<'_>> = layer_path(&mut app).complete();
            let MaybeInvalidated::NotInvalidated(layer) = stayed.to_maybe_invalidated() else {
                panic!("stopping here re-establishes the path");
            };
            assert_eq!(layer.get().nav.hits, 7);
        }

        {
            let left: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
            let MaybeInvalidated::Invalidated(completed) = left.to_maybe_invalidated() else {
                panic!("going above stays a leave");
            };
            let Stop::Up(root) = completed.into_inner() else {
                panic!("the leave still points above the layer");
            };
            root.hits = 3;
        }
        assert_eq!(app.hits, 3);
    }

    /// A leave from the root can only have stopped at the root, so the fold
    /// there has one answer.
    #[test]
    fn a_leave_from_the_root_never_invalidates_it() {
        let mut app = tree(0, 0);
        {
            let folded: Completed<AppPath<'_>> = Completed::from(&mut app);
            let MaybeInvalidated::NotInvalidated(root) = folded.to_maybe_invalidated() else {
                panic!("the root's own leave leaves the root standing");
            };
            root.hits = 4;
        }
        assert_eq!(app.hits, 4);
    }

    /// What a node ends its dispatch with: either branch completes to the leave
    /// its caller reads.
    #[test]
    fn either_branch_completes() {
        let mut app = tree(7, 0);
        {
            let here: Completed<LayerPath<'_>> =
                MaybeInvalidated::NotInvalidated(layer_path(&mut app)).complete();
            let Stop::Here(layer) = here.into_inner() else {
                panic!("a standing path completes where it stands");
            };
            assert_eq!(layer.get().nav.hits, 7);
        }

        {
            let left: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
            let up: Completed<LayerPath<'_>> = MaybeInvalidated::Invalidated(left).complete();
            let Stop::Up(root) = up.into_inner() else {
                panic!("an invalidated state completes to the leave it holds");
            };
            root.hits = 5;
        }
        assert_eq!(app.hits, 5);
    }

    /// A sibling descent from a standing state: the path is lent, and the
    /// sibling's own outcome is the state.
    #[test]
    fn descend_lends_a_standing_path() {
        let mut app = tree(7, 0);
        let out = MaybeInvalidated::NotInvalidated(layer_path(&mut app)).descend(|mut layer| {
            layer.get_mut().nav.hits = 8;
            MaybeInvalidated::NotInvalidated(layer)
        });
        let MaybeInvalidated::NotInvalidated(layer) = out else {
            panic!("a standing path stays standing when the sibling stays");
        };
        assert_eq!(layer.get().nav.hits, 8);
    }

    /// A leave that stopped here still descends the sibling, and the node stays
    /// invalidated whatever the sibling does.
    #[test]
    fn descend_recovers_a_stopped_here_leave_and_stays_invalidated() {
        let mut app = tree(7, 0);
        let stopped_here: MaybeInvalidated<LayerPath<'_>> =
            MaybeInvalidated::Invalidated(layer_path(&mut app).complete());
        let out = stopped_here.descend(|mut layer| {
            layer.get_mut().nav.hits = 8;
            MaybeInvalidated::NotInvalidated(layer)
        });
        let MaybeInvalidated::Invalidated(completed) = out else {
            panic!("a stopped-here leave is preserved past the sibling");
        };
        let Stop::Here(layer) = completed.into_inner() else {
            panic!("the preserved leave still stops here");
        };
        assert_eq!(layer.get().nav.hits, 8);
    }

    /// A sibling that leaves higher replaces a stopped-here leave.
    #[test]
    fn descend_lets_a_higher_leave_replace_a_stopped_here_one() {
        let mut app = tree(0, 0);
        {
            let stopped_here: MaybeInvalidated<LayerPath<'_>> =
                MaybeInvalidated::Invalidated(layer_path(&mut app).complete());
            let out = stopped_here
                .descend(|layer| MaybeInvalidated::Invalidated(layer.into_parent().complete()));
            let MaybeInvalidated::Invalidated(completed) = out else {
                panic!("the higher leave is the state");
            };
            let Stop::Up(root) = completed.into_inner() else {
                panic!("the higher leave went above the layer");
            };
            root.hits = 6;
        }
        assert_eq!(app.hits, 6);
    }

    /// A leave that went above skips the sibling: it is never descended.
    #[test]
    fn descend_skips_when_the_leave_went_above() {
        let mut app = tree(0, 0);
        {
            let gone: MaybeInvalidated<LayerPath<'_>> =
                MaybeInvalidated::Invalidated(layer_path(&mut app).into_parent().complete());
            let out = gone.descend(|_layer| panic!("the sibling is not descended"));
            let MaybeInvalidated::Invalidated(completed) = out else {
                panic!("the leave forwards");
            };
            let Stop::Up(root) = completed.into_inner() else {
                panic!("the leave still points above the layer");
            };
            root.hits = 9;
        }
        assert_eq!(app.hits, 9);
    }
}

#[cfg(test)]
mod ancestors_through_a_leave_tests {
    use crate::{
        Completed, CompletesTo, HasAncestor, HasStop, IntoAncestor, MaybeInvalidated, PathMut,
    };

    struct App {
        hits: u32,
        layer: Layer,
    }
    struct Layer {
        hits: u32,
        nav: Nav,
    }
    struct Nav {
        hits: u32,
        deep: Deep,
    }
    struct Deep {
        hits: u32,
    }

    type AppPath<'a> = &'a mut App;
    type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
    type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;
    type DeepPath<'a> = PathMut<Deep, NavPath<'a>>;

    const fn tree() -> App {
        App {
            hits: 0,
            layer: Layer {
                hits: 0,
                nav: Nav {
                    hits: 0,
                    deep: Deep { hits: 0 },
                },
            },
        }
    }

    fn layer_path(app: &mut App) -> LayerPath<'_> {
        PathMut::from_fn(app, |a| &mut a.layer, |a| &a.layer)
    }

    fn nav_path(app: &mut App) -> NavPath<'_> {
        PathMut::from_fn(
            layer_path(app),
            |lp| &mut lp.get_mut().nav,
            |lp| &lp.get().nav,
        )
    }

    fn deep_path(app: &mut App) -> DeepPath<'_> {
        PathMut::from_fn(
            nav_path(app),
            |np| &mut np.get_mut().deep,
            |np| &np.get().deep,
        )
    }

    /// Fails to compile if either reach is short at any depth, on either carrier.
    #[test]
    fn completed_and_state_reach_the_root_at_every_depth() {
        const fn reaches<'a, T: HasAncestor<AppPath<'a>> + IntoAncestor<AppPath<'a>>>() {}
        reaches::<Completed<AppPath<'_>>>();
        reaches::<Completed<LayerPath<'_>>>();
        reaches::<Completed<NavPath<'_>>>();
        reaches::<Completed<DeepPath<'_>>>();
        reaches::<MaybeInvalidated<AppPath<'_>>>();
        reaches::<MaybeInvalidated<LayerPath<'_>>>();
        reaches::<MaybeInvalidated<NavPath<'_>>>();
        reaches::<MaybeInvalidated<DeepPath<'_>>>();
    }

    /// Wherever a leave stopped, the root is still in it: the `Here` arm walks the
    /// standing path up, the `Up` arm asks the leave above.
    #[test]
    fn a_leave_holds_the_root_wherever_it_stopped() {
        let mut app = tree();
        {
            let stopped_at_nav: Completed<NavPath<'_>> = nav_path(&mut app).complete();
            assert_eq!(stopped_at_nav.ancestor::<AppPath<'_>>().hits, 0);
            stopped_at_nav.into_ancestor::<AppPath<'_>>().hits = 1;
        }
        assert_eq!(app.hits, 1);

        {
            let stopped_at_layer: Completed<NavPath<'_>> =
                nav_path(&mut app).into_parent().complete();
            stopped_at_layer.into_ancestor::<AppPath<'_>>().hits = 2;
        }
        assert_eq!(app.hits, 2);

        {
            let peeled_to_root: Completed<NavPath<'_>> =
                nav_path(&mut app).into_parent().into_parent().complete();
            peeled_to_root.into_ancestor::<AppPath<'_>>().hits = 3;
        }
        assert_eq!(app.hits, 3);

        // One level deeper, so the recursion runs through two `Up` frames rather
        // than one, and the levels it passes are untouched on the way.
        {
            let from_deep: Completed<DeepPath<'_>> = deep_path(&mut app).complete();
            assert_eq!(from_deep.ancestor::<AppPath<'_>>().layer.hits, 0);
            assert_eq!(from_deep.ancestor::<AppPath<'_>>().layer.nav.hits, 0);
            assert_eq!(from_deep.ancestor::<AppPath<'_>>().layer.nav.deep.hits, 0);
            from_deep.into_ancestor::<AppPath<'_>>().hits = 4;
        }
        assert_eq!(app.hits, 4);
    }

    /// One handler, no match on the state, bound at every depth: what it means is
    /// that the dispatch ends at the root, so it re-roots the leave either way.
    #[test]
    fn one_root_handler_serves_both_branches_at_every_depth() {
        fn go_root<'a, P>(state: MaybeInvalidated<P>) -> Completed<P>
        where
            P: HasStop,
            MaybeInvalidated<P>: IntoAncestor<AppPath<'a>>,
            AppPath<'a>: CompletesTo<P>,
        {
            let root: AppPath<'a> = state.into_ancestor();
            root.hits += 1;
            root.complete()
        }

        let mut app = tree();
        {
            let standing = go_root(MaybeInvalidated::NotInvalidated(nav_path(&mut app)));
            assert_eq!(standing.into_ancestor::<AppPath<'_>>().hits, 1);
        }
        {
            let left: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
            let invalidated = go_root(MaybeInvalidated::Invalidated(left));
            assert_eq!(invalidated.into_ancestor::<AppPath<'_>>().hits, 2);
        }
        {
            let at_the_root = go_root(MaybeInvalidated::NotInvalidated(&mut app));
            assert_eq!(at_the_root.into_ancestor::<AppPath<'_>>().hits, 3);
        }
        assert_eq!(app.hits, 3);
    }

    /// Distance zero: a leave reaches its own origin iff it stopped there.
    #[test]
    fn try_at_distance_zero_recovers_a_here_stop() {
        let mut app = tree();
        {
            let stopped: Completed<NavPath<'_>> = nav_path(&mut app).complete();
            let Ok(mut nav) = stopped.try_into_ancestor::<NavPath<'_>>() else {
                panic!("a leave that stopped at nav still holds nav");
            };
            nav.get_mut().hits = 5;
        }
        assert_eq!(app.layer.nav.hits, 5);

        {
            let peeled: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
            let Err(back) = peeled.try_into_ancestor::<NavPath<'_>>() else {
                panic!("a leave that peeled past nav cannot hand nav back");
            };
            // The leave came back whole, so the caller can still forward it.
            back.into_ancestor::<AppPath<'_>>().hits = 6;
        }
        assert_eq!(app.hits, 6);
    }

    /// A mid ancestor is alive exactly when the leave stopped at or below it.
    #[test]
    fn try_reaches_a_mid_ancestor_iff_the_leave_stopped_at_or_below_it() {
        let mut app = tree();
        {
            let stopped_below: Completed<NavPath<'_>> = nav_path(&mut app).complete();
            let Ok(mut layer) = stopped_below.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("stopping at nav leaves the layer above it standing");
            };
            layer.get_mut().hits = 1;
        }
        assert_eq!(app.layer.hits, 1);

        {
            let stopped_at: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
            let Ok(mut layer) = stopped_at.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("stopping exactly at the layer recovers it");
            };
            layer.get_mut().hits = 2;
        }
        assert_eq!(app.layer.hits, 2);

        {
            let peeled: Completed<NavPath<'_>> =
                nav_path(&mut app).into_parent().into_parent().complete();
            let Err(back) = peeled.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("a leave past the layer cannot hand it back");
            };
            back.into_ancestor::<AppPath<'_>>().hits = 3;
        }
        assert_eq!(app.hits, 3);
    }

    /// The root is the one target that is always still there.
    #[test]
    fn try_to_the_root_always_succeeds() {
        let mut app = tree();
        {
            let peeled: Completed<LayerPath<'_>> = layer_path(&mut app).into_parent().complete();
            let Ok(root) = peeled.try_into_ancestor::<AppPath<'_>>() else {
                panic!("the root outlives every leave");
            };
            root.hits = 7;
        }
        assert_eq!(app.hits, 7);
    }

    /// On the state: the standing branch reaches every chain ancestor, and the
    /// invalidated one answers with its leave, giving the state back on a miss.
    #[test]
    fn try_on_the_state_covers_both_branches() {
        let mut app = tree();
        {
            let standing: MaybeInvalidated<NavPath<'_>> =
                MaybeInvalidated::NotInvalidated(nav_path(&mut app));
            let Ok(mut layer) = standing.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("a standing path reaches its own ancestors");
            };
            layer.get_mut().hits = 8;
        }
        assert_eq!(app.layer.hits, 8);

        {
            let leave: Completed<NavPath<'_>> =
                nav_path(&mut app).into_parent().into_parent().complete();
            let invalidated: MaybeInvalidated<NavPath<'_>> = MaybeInvalidated::Invalidated(leave);
            let Err(back) = invalidated.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("the leave went above the layer");
            };
            let MaybeInvalidated::Invalidated(forwardable) = back else {
                panic!("the state comes back as it went in");
            };
            forwardable.into_ancestor::<AppPath<'_>>().hits = 9;
        }
        assert_eq!(app.hits, 9);
    }

    /// Distance two, the macro impl's `Here` arm: the standing path walks up.
    #[test]
    fn try_here_arm_at_macro_depth() {
        let mut app = tree();
        {
            let stopped_at_deep: Completed<DeepPath<'_>> = deep_path(&mut app).complete();
            let Ok(mut layer) = stopped_at_deep.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("stopping at deep leaves the layer two levels up standing");
            };
            layer.get_mut().hits = 11;
        }
        assert_eq!(app.layer.hits, 11);
    }

    /// Distance two, the macro impl's `Err` path: each distance rebuilds the leave
    /// on the way back out, so what the caller gets is the leave it started with.
    #[test]
    fn try_err_rebuilds_through_the_macro() {
        let mut app = tree();
        {
            let peeled: Completed<DeepPath<'_>> = deep_path(&mut app)
                .into_parent()
                .into_parent()
                .into_parent()
                .complete();
            let Err(back) = peeled.try_into_ancestor::<LayerPath<'_>>() else {
                panic!("a leave peeled to the root cannot hand the layer back");
            };
            back.into_ancestor::<AppPath<'_>>().hits = 12;
        }
        assert_eq!(app.hits, 12);
    }

    /// The shared reach, on both branches of the state.
    #[test]
    fn the_state_reads_the_root_on_both_branches() {
        let mut app = tree();
        app.hits = 4;
        {
            let standing: MaybeInvalidated<NavPath<'_>> =
                MaybeInvalidated::NotInvalidated(nav_path(&mut app));
            assert_eq!(standing.ancestor::<AppPath<'_>>().hits, 4);
        }
        {
            let left: Completed<NavPath<'_>> = nav_path(&mut app).into_parent().complete();
            let invalidated: MaybeInvalidated<NavPath<'_>> = MaybeInvalidated::Invalidated(left);
            assert_eq!(invalidated.ancestor::<AppPath<'_>>().hits, 4);
        }
    }
}
