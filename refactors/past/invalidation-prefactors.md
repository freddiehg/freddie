# Invalidation prefactors: changes 0–4, landed

The landed prefactors of `refactors/pending/invalidation.md`, recorded here as implemented; the model they serve and the remaining changes (5, 6) stay in that doc. Commits: fa2f155 (change 0), 74e8077 (change 1), 7b19f86 (change 2), 2f6c083 (change 3), 62345c7 (change 4).

## 0 — the standalone `DispatchIntoParent` rename

## 1 — laserbeam: `MaybeInvalidated`, the conversions

`MaybeInvalidated` is active-path semantics, meaningful in a tree with no bindings, so it lives beside `Stop`:

```rust
/// Have we destroyed the path we need?
pub enum MaybeInvalidated<P: HasStop> {
    /// No: here it is.
    NotInvalidated(P),
    /// Yes: the completed leave, ready to forward. `Here` inside it means the
    /// leave stopped at this path, so the path is recoverable.
    Invalidated(Completed<P>),
}

impl<P: HasStop> MaybeInvalidated<P>
where
    P: Complete<P>,
{
    pub fn complete(self) -> Completed<P> {
        match self {
            Self::NotInvalidated(path) => path.complete(),
            Self::Invalidated(completed) => completed,
        }
    }
}

/// A child of the root: the Up payload is the bare root path.
impl<'a, N, R> Stop<PathMut<N, &'a mut R>, &'a mut R> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<&'a mut R> {
        match self {
            Stop::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Stop::Up(root) => MaybeInvalidated::Invalidated(root.complete()),
        }
    }
}

/// A child of a non-root: the Up payload is this node's own Completed.
impl<N, N2, Q: Above> Stop<PathMut<N, PathMut<N2, Q>>, Completed<PathMut<N2, Q>>> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<PathMut<N2, Q>> {
        match self {
            Stop::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Stop::Up(rest) => MaybeInvalidated::Invalidated(rest),
        }
    }
}
```

`HasStop` gained one associated fn, implemented by its two existing impls; the wrappers use it to fold a handler's returned `Completed` back into the state:

```rust
pub trait HasStop: Sized {
    type Stop;
    /// A handler's returned leave, as the state it leaves behind.
    fn to_maybe_invalidated(completed: Completed<Self>) -> MaybeInvalidated<Self>;
}

impl<N, P: Above> HasStop for PathMut<N, P> {
    type Stop = Stop<PathMut<N, P>, P::Up>;
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

impl<P: HasStop> Completed<P> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<P> {
        P::to_maybe_invalidated(self)
    }
}
```

One conversion was added, a general normalization of a bare-root Up payload:

```rust
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

Unit tests on all of it landed in laserbeam with this change.

## 2 — bind: `AscendState`, `exclusive`, `Claim::reborrow`; free `dispatch` returns `(Vec<E>, bool)`

```rust
/// What every scheduled handler receives beside the event. One lifetime: the
/// claim rides by value, reborrowed per item.
pub struct AscendState<'a, P: ::laserbeam::HasStop> {
    claim: Claim<'a>,
    pub state: ::laserbeam::MaybeInvalidated<P>,
}

impl<'a, P: ::laserbeam::HasStop> AscendState<'a, P> {
    pub fn new(state: ::laserbeam::MaybeInvalidated<P>, claim: Claim<'a>) -> Self {
        Self { claim, state }
    }

    /// `Some(())`: you won the claim. `None`: someone already has it.
    pub fn claim(&mut self) -> Option<()> {
        self.claim.try_take()
    }

    pub fn complete(self) -> ::laserbeam::Completed<P>
    where
        P: ::laserbeam::Complete<P>,
    {
        self.state.complete()
    }
}

impl<'c> Claim<'c> {
    /// The per-item reborrow the generated code hands to each `AscendState`.
    pub fn reborrow(&mut self) -> Claim<'_> {
        Claim { slot: &mut *self.slot }
    }
}

/// The claim gate, shape-preserving: the handler runs iff the claim is won;
/// otherwise it completes the state where it stands.
pub fn exclusive<Ev, Snap, P, E, H>(
    handler: H,
) -> impl for<'a> FnOnce(Ev, Snap, AscendState<'a, P>) -> (Vec<E>, ::laserbeam::Completed<P>)
where
    P: ::laserbeam::HasStop + ::laserbeam::Complete<P>,
    H: for<'a> FnOnce(Ev, Snap, AscendState<'a, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
{
    move |ev, snap, mut st| match st.claim() {
        Some(()) => handler(ev, snap, st),
        None => (Vec::new(), st.complete()),
    }
}
```

The free `dispatch`, before:

```rust
let _completed = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
if claim.is_taken() || !effs.is_empty() {
    Some(effs)
} else {
    None
}
```

After — effects are always returned and always performed; whether the key was handled is the claim's answer and nothing else's, so an unclaimed key with post effects (rearm) still passes through to the OS:

```rust
pub fn dispatch<'a, M, N, E>(path: N::Path<'a>, event: &M::Event) -> (Vec<E>, bool)
where
    M: Bindings<Output = Vec<E>>,
    N: Dispatch<M> + 'a,
    N::Path<'a>: ::laserbeam::HasStop,
{
    let mut effs: Vec<E> = Vec::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _completed = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    (effs, claim.is_taken())
}
```

Consumers (mercury's loop, `SimpleRunner`, tests) perform the effects unconditionally and use the bool for pass-through. Call sites name the effect type the two-argument call cannot infer: `dispatch::<Demo, App, _>(&mut app, &event)`. `Mercury::handle` returns `(Vec<MercuryEffect>, bool)` with the layer rearm gated on the claim; `SimpleRunner::next` returns `Option<(Vec<E>, bool)>` and `process_event` the pair. `crates/bind/tests/ascend_state.rs` pins the gate: it runs when free, is skipped when taken, forwards an invalidated state, and an ungated item runs with the claim gone.

## 3 — bind_macro: opts before descent, source order, synthesized `|_, _| ()` pre

Before, each check built its trigger inline, after the recursion, inside `dispatch_impl`'s `checks`:

```rust
if let ::core::option::Option::Some(ev) =
    ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
{
    let trigger = #trigger;
    if ::bind::EventTrigger::is_matching(&trigger, ev) {
        if let ::core::option::Option::Some(()) = claim.try_take() {
            *effs = ::core::iter::Iterator::collect(
                ::core::iter::IntoIterator::into_iter(
                    #handler(ev, ::bind::Node { parent: path, data: () }),
                ),
            );
            return ::core::option::Option::None;
        }
    }
}
```

After: one opt local per scheduled attribute, emitted before the descent, numbered in source order across all kinds; the checks consume the opts but keep their firing form, so the change is behavior-visible only in when triggers and pres read state (pre-descent, which is the point):

```rust
let opt_N = match ::core::convert::TryFrom::try_from(event) {
    ::core::result::Result::Ok(ev) => {
        let trigger = #trigger;
        if ::bind::EventTrigger::is_matching(&trigger, ev) {
            ::core::option::Option::Some((ev, (#pre)(ev, &path))) // #pre: written, or synthesized |_, _| ()
        } else {
            ::core::option::Option::None
        }
    }
    ::core::result::Result::Err(_) => ::core::option::Option::None,
};
```

## 4 — route enums: the Up half

A multi-parent child's parent slot is a route enum, and the consumer hand-writes both directions: the route enum, an Up enum, and a one-line `Above` impl. The Up payload records which route the leave took and how far it went, one variant per parent, each carrying that parent's own `Completed`:

```rust
// tests/common/mod.rs, beside TitleParent
pub enum TitleParentUp<'a> {
    Album(Completed<AlbumPath<'a>>),
    Song(Completed<SongPath<'a>>),
}

impl<'a> Above for TitleParent<'a> {
    type Up = TitleParentUp<'a>;
}
```

Everything else composes from existing impls, so laserbeam changed not at all: `TitlePath: HasStop` via the blanket `impl<N, P: Above> HasStop for PathMut<N, P>` (its `Stop` is `Stop<TitlePath, TitleParentUp>`), staying put via the blanket zero-peel `Complete`, `Completed::up` already accepts `Par::Up`, and a root parent's variant would carry `Completed<&'a mut R>`. Route enums nest: `TitleParent: Above` makes `TitlePath: Above`, so a child of `Title` would be an ordinary edge.

`compile_fail/route_parent_completed.rs` pinned the opposite and was deleted (path-peel-complete.md's rule 7 is superseded); the shape pin replaced it, in `tests/complete.rs`:

```rust
fn title_shapes<'a>(c: Completed<TitlePath<'a>>) {
    let stop: Stop<TitlePath<'a>, TitleParentUp<'a>> = c.into_inner();
    if let Stop::Up(TitleParentUp::Album(rest)) = stop {
        let _: Stop<AlbumPath<'a>, MediaPath<'a>> = rest.into_inner();
    }
}
```

The macro wiring (the `up = ..` attribute argument, the parent-side fold, `title_home`) is invalidation change 5's.
