# Ancestors through a completed leave

Not started. The baseline is laserbeam as it stands after invalidation.md's change 1 (`Completed`, `MaybeInvalidated`, `Above`, `HasStop`, all on disk); nothing here waits on invalidation.md's changes 4–6. Every impl and every test below was compile-checked and run green in the design scratch, at the workspace's clippy levels. The consumer snippets use the handler signature that arrives with invalidation change 5, so mercury adoption rides that migration; the laserbeam changes ship on their own.

## Model

Every inhabitant of `Completed<P>` still holds the root. A leave that stopped at a path holds that path, and the root is an ancestor of every path; a leave that peeled to the root holds the root bare. The root is the only ancestor with that property: `Completed<P>` erases where the leave stopped (that erasure is what lets one handler return the same type from every branch), so any shallower ancestor is absent from the went-to-the-root inhabitant.

Two reaches follow, and they are different shapes:

- To the root: total. `Completed<P>` and `MaybeInvalidated<P>` implement the existing `HasAncestor` and `IntoAncestor` with the root path as the target. A handler whose meaning is "end at the root" stops matching the state: `st.state.into_ancestor()` reaches the root on both branches, at every depth.
- To any other chain ancestor: fallible. A new trait, `TryIntoAncestor<Target>`, answers "is the target still alive on this leave" with `Result<Target, Self>`: `Ok` iff the leave stopped at or below the target, so the target is standing; `Err` gives the whole value back, because the caller still has to return a `Completed<P>` and would otherwise have destroyed the very leave it must forward. (`Option` is the wrong return type for exactly that reason: `None` would consume the leave and hand back nothing.) To the root, `try_into_ancestor` always returns `Ok`.

The consuming forms are for handlers that end at the target. `into_ancestor` to the root consumes the leave, and the root rebuilds only its own leave, so the handler answers `root.complete()`, a to-the-root leave, wherever the consumed one had stopped. `try_into_ancestor`'s `Ok` arm likewise ends at the target: the handler mutates the recovered path and answers `Completed::up(target.complete())`, a leave to the target. Reading an ancestor without deciding anything is `ancestor` (by shared reference, root only) or a hand-match on `into_inner`, which both stay what they are.

Route enums stay uncrossed. The impls recurse on the `PathMut`/root shape of the parent slot, and a route enum is neither, so the bounds fail at the route edge, exactly where `HasAncestor` on paths already stops.

## Change 1 — laserbeam: the root through a leave

Six impls and four sugar methods. The recursive impls ground through the reflexive `HasAncestor<T> for T` when the parent is the root (its `Up` is the bare root path) and through themselves when it is a path (its `Up` is a `Completed`).

```rust
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

/// The state after a descent holds the root on both branches: through the
/// standing path, or through the leave that replaced it.
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
```

None of these overlaps the reflexive `HasAncestor<T> for T`: that impl's target equals its `Self`, and neither `Completed<_>` nor `MaybeInvalidated<_>` unifies with `&'a mut R`.

The turbofish sugar, for the same reason `PathMut` has it (the trait method takes no generic arguments): `ancestor` and `into_ancestor` join the existing `impl<P: HasStop> Completed<P>` block, and `MaybeInvalidated` gets a new `impl<P: HasStop>` block beside its `complete` one (whose `Complete<P>` bound the sugar does not need).

```rust
impl<P: HasStop> Completed<P> {
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
}
```

## Change 2 — laserbeam: `TryIntoAncestor`

One trait, four impl shapes: distance zero, the two distance-one terminals (root and non-root, split because the `Up` payload's type differs there), and one macro-emitted impl per distance from two to twelve, mirroring how `Stop::to_maybe_invalidated` and `complete_impls` already index by shape. The impls are disjoint by the same occurs-check argument as `HasAncestor`'s.

```rust
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
```

The sugar, one method added to each of change 1's two inherent blocks:

```rust
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
```

## Consumer shape (invalidation change 5's handler signature)

The demo's `go_home` (`A`/`B` from invalidation.md), before:

```rust
fn go_home<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, BPath<'x>>,
) -> (Vec<DemoEffect>, Completed<BPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(b) => (vec![], b.into_parent().complete()),
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

After, branch-free and depth-generic, binding at every node of the tree; on the invalidated branch it re-roots the leave, because this handler's meaning is that the dispatch ends at the root wherever the prior leave stopped:

```rust
fn go_home<'x, P>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, P>,
) -> (Vec<DemoEffect>, Completed<P>)
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<APath<'x>>,
    APath<'x>: Complete<P>,
{
    (vec![], st.state.into_ancestor::<APath<'x>>().complete())
}
```

Mercury's root-consuming handlers (`and_go_home` and its family) take this same shape when change 5 migrates them: the `P: IntoAncestor<MercuryPath<'a>>` bound on the path becomes `MaybeInvalidated<P>: IntoAncestor<MercuryPath<'a>>` on the state.

A handler that ends at a mid-level ancestor when it survived, on the `App → Layer → Nav` tree (the fixture below): the leave from `Nav`'s own descent may have stopped at `Layer` or peeled past it, and `try_into_ancestor` is the one call that tells the two apart while keeping the leave.

```rust
fn back_to_layer<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, NavPath<'x>>,
) -> (Vec<DemoEffect>, Completed<NavPath<'x>>) {
    match st.state.try_into_ancestor::<LayerPath<'x>>() {
        Ok(mut layer) => {
            layer.get_mut().hits += 1;
            (vec![], Completed::up(layer.complete()))
        }
        Err(state) => (vec![], state.complete()),
    }
}
```

## Tests

A new `#[cfg(test)]` mod in laserbeam's `lib.rs`, on a three-level fixture with a `hits` counter at each level (`App { hits, layer }`, `Layer { hits, nav }`, `Nav { hits }`, with `AppPath`/`LayerPath`/`NavPath` and the `layer_path`/`nav_path` builders the existing test mods use). All of the following ran green in the design scratch.

- `completed_and_state_reach_the_root_at_every_depth`: a const fn bounded on `HasAncestor<AppPath> + IntoAncestor<AppPath>` instantiated at `Completed` and `MaybeInvalidated` of all three depths.
- `a_leave_holds_the_root_wherever_it_stopped`: `ancestor` reads and `into_ancestor` writes the root through leaves stopped at `Nav`, at `Layer`, and at the root.
- `one_root_handler_serves_both_branches_at_every_depth`: the depth-generic root handler, written out:

```rust
fn go_root<'a, P>(state: MaybeInvalidated<P>) -> Completed<P>
where
    P: HasStop,
    MaybeInvalidated<P>: IntoAncestor<AppPath<'a>>,
    AppPath<'a>: Complete<P>,
{
    let root: AppPath<'a> = state.into_ancestor();
    root.hits += 1;
    root.complete()
}
```

driven at `NotInvalidated(nav_path)`, at `Invalidated` of a one-peel leave, and at the root itself, asserting the returned leave unwraps to the root each time.

- `try_at_distance_zero_recovers_a_here_stop`: `Ok` on a leave that stopped at `Nav`, `Err` handing the leave back on one that peeled.
- `try_reaches_a_mid_ancestor_iff_the_leave_stopped_at_or_below_it`: target `LayerPath` from `Completed<NavPath>`: `Ok` when stopped at `Nav` (the target is above the stop), `Ok` when stopped at `Layer` (exactly the target), `Err` when peeled to the root, with the returned leave still unwrapping to the root.
- `try_to_the_root_always_succeeds`: target `AppPath` from a fully peeled leave.
- `try_on_the_state_covers_both_branches`: `MaybeInvalidated<NavPath>`: `Ok(LayerPath)` on the standing branch; `Err` returning the state, still `Invalidated` and still holding the forwardable leave, on the peeled one.

## Ordered changes

### 1 — laserbeam: the six root-reach impls, the four sugar methods, their tests

### 2 — laserbeam: `TryIntoAncestor` (trait, distance-zero impl, the two distance-one impls, the macro, the `MaybeInvalidated` impl), the two sugar methods, their tests
