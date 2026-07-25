# Invalidation: descent schedules, ascent runs posts

Not done. Prefactor `path-peel-complete.md`: landed. Invalidation change "Claim + effs sink, drop ControlFlow" (22e5580): landed. Changes 0–3: landed (fa2f155, 74e8077, 7b19f86, 2f6c083); every "before" below is the code on disk after 2f6c083. Changes 1–3's types and generated shapes were compile-checked in the design scratch; the route-enum section is not yet. Change 5 is blocked until `derived-levels.md` is settled.

Descent schedules which posts run. That set is final. Ascent runs every scheduled post.

## Model

Every place node's dispatch returns `Completed<Self::Path<'a>>`, root and route-parented nodes included (derived levels: `derived-levels.md`). Between nodes, the protocol is the child's `Completed` and its `Stop` arms. Inside a node, dispatch is one linear, kind-blind body:

```text
opts        one local per scheduled attribute, source order, snapped before descent
descend     child dispatch → .into_inner().to_maybe_invalidated() → AscendState
scheduled   one identical block per item: call, extend effs, rebind the state
complete    state.complete()
```

`MaybeInvalidated<P>` answers one question, have we destroyed the path we need:

```text
NotInvalidated(P)          no — here it is
Invalidated(Completed<P>)  yes — the completed leave, ready to forward;
                           Here inside it: the leave stopped at this path
```

Every scheduled item is a pre_post; `#[bind]` and `#[post]` are the ones whose pre is `|_, _| ()`. So every handler has literally one signature, separate parameters, never a tuple, returning the call to `.complete()`:

```rust
FnOnce(&Ev, Snap, AscendState<'a, P>) -> (Vec<E>, Completed<P>)  // Snap = () without a pre
```

A handler matches the state totally, no helpers, `into_parent` the only way up; staying put is `st.complete()`:

```rust
match st.state {
    MaybeInvalidated::NotInvalidated(b) => {
        let parent = b.into_parent();
        (vec![], parent.complete())
    }
    MaybeInvalidated::Invalidated(c) => (vec![], c),
}
```

`Invalidated` is unforgeable, since `Completed` has no public constructor; `NotInvalidated` accepts only a path of exactly type `P`. The generated code folds every returned `Completed` back into the state, separately, after each item (`completed.to_maybe_invalidated()`: `Here` re-establishes the path as `NotInvalidated`, `Up` stays a forwarded leave), so the state evolves through the schedule — with `#[post(a => b, c => d)]`, `b` can receive `NotInvalidated`, leave, and `d` then receives `Invalidated`. Every scheduled item runs; a leave is data, not control flow, and nothing early-returns. A consequence of the fold: after any item runs, the state reflects that item's answer, not the descent's, so a post keyed on what the descent did (the return-home cancel) is scheduled before any bind.

`#[bind(X => foo)]` desugars to `#[pre_post(X => (|_, _| (), exclusive(foo)))]` and `#[post(X => f)]` to `#[pre_post(X => (|_, _| (), f))]`; the macro looks inside no rhs. `exclusive` is shape-preserving and means not claimed: it calls `foo` iff the claim is won and otherwise completes the state where it stands, so its output is the same scheduled shape as everything else. The claim's win is not part of any signature, because winning the claim does not imply `NotInvalidated` (a post can leave without claiming); what each state branch means is the handler's business.

"Invalidated" means off the active path: focus left it. Whether state was also replaced is the handler's business (an enum layer usually swaps; a struct field persists).

## laserbeam additions (all of change 1)

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

`HasStop` gains one associated fn, implemented by its two existing impls; the wrappers use it to fold a handler's returned `Completed` back into the state:

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

One conversion is added (landed with change 1; a general normalization of a bare-root Up payload):

```rust
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

## bind additions (change 2, with the free `dispatch` delta in the macro section)

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

## Route enums

A multi-parent child's parent slot is a route enum, and the consumer hand-writes both directions: the route enum (exists today) plus an Up enum and a one-line `Above` impl (change 4). The Up payload records which route the leave took and how far it went, one variant per parent, each carrying that parent's own `Completed`:

```rust
// tests/common/mod.rs, beside TitleParent (change 4)
pub enum TitleParentUp<'a> {
    Album(Completed<AlbumPath<'a>>),
    Song(Completed<SongPath<'a>>),
}

impl<'a> Above for TitleParent<'a> {
    type Up = TitleParentUp<'a>;
}
```

Everything else composes from existing impls, so laserbeam changes not at all: `TitlePath: HasStop` via the blanket `impl<N, P: Above> HasStop for PathMut<N, P>` (its `Stop` is `Stop<TitlePath, TitleParentUp>`), staying put via the blanket zero-peel `Complete`, `Completed::up` already accepts `Par::Up`, and a root parent's variant would carry `Completed<&'a mut R>`. Route enums nest: `TitleParent: Above` makes `TitlePath: Above`, so a child of `Title` would be an ordinary edge. `compile_fail/route_parent_completed.rs` pins the opposite and is deleted in change 4 (path-peel-complete.md's rule 7 is superseded); the shape pin replaces it, in `tests/complete.rs`:

```rust
fn title_shapes<'a>(c: Completed<TitlePath<'a>>) {
    let stop: Stop<TitlePath<'a>, TitleParentUp<'a>> = c.into_inner();
    if let Stop::Up(TitleParentUp::Album(rest)) = stop {
        let _: Stop<AlbumPath<'a>, MediaPath<'a>> = rest.into_inner();
    }
}
```

The macro is told both enum names, since it can derive neither (change 5):

```rust
#[resolve_into(parent = TitleParent, up = TitleParentUp)]
pub title: Title,

#[derive(Bind)]
#[node(parent = TitleParent)]
#[binds(Demo)]
#[bind(Keyboard("t") => on_title, Keyboard("home") => title_home)]
pub struct Title {
    pub hits: u32,
}
```

`into_parent()` on a `TitlePath` yields the route enum, which has no `into_parent` of its own, so a leaving handler matches it and wraps one `Up` level by hand. Both arms are live, one per route; `IntoAncestor` / `HasAncestor` still do not cross route enums, so the generic go-home handlers do not bind here:

```rust
/// Title's leave, on `home`: out through whichever route is live, to the root.
fn title_home<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, TitlePath<'x>>,
) -> (Vec<usize>, Completed<TitlePath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(title) => {
            let up = match title.into_parent() {
                TitleParent::Album(album) => TitleParentUp::Album(album.into_parent().complete()),
                TitleParent::Song(song) => TitleParentUp::Song(song.into_parent().complete()),
            };
            (vec![], Completed::up(up))
        }
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}
```

The parent-side fold. A route edge's `#state` recovers the variant the descent constructed; the `unreachable!()`s assert exactly what `Edge::recover_parent` and the multi-parent projections assert today, relocated into the fold. In Album's generated dispatch:

```rust
let mut state = match ::laserbeam::Completed::into_inner(
    <Title as ::bind::Dispatch<Demo>>::dispatch(#child_path, event, effs, claim),
) {
    ::laserbeam::Stop::Here(title) => {
        let TitleParent::Album(album) = ::bind::HasParent::into_parent(title) else {
            ::core::unreachable!()
        };
        ::laserbeam::MaybeInvalidated::NotInvalidated(album)
    }
    ::laserbeam::Stop::Up(up) => {
        let TitleParentUp::Album(c) = up else { ::core::unreachable!() };
        ::laserbeam::MaybeInvalidated::Invalidated(c)
    }
};
```

No into-parent dispatch question arises for a route-parented node: change 5 deletes the place `dispatch_into_parent_impl` outright, for every place (`derived-levels.md`).

## Landed baseline

`bind/src/lib.rs` holds `Claim` (with `reborrow`), `AscendState`, `exclusive`, and the free `dispatch` returning `(Vec<E>, bool)` — changes 1–2, landed. Call sites name the effect type the two-argument call cannot infer: `dispatch::<Demo, App, _>(&mut app, &event)`. `Mercury::handle` returns `(Vec<MercuryEffect>, bool)` with the layer rearm gated on the claim; `SimpleRunner::next` returns `Option<(Vec<E>, bool)>` and `process_event` the pair. `Dispatch` and `DispatchIntoParent` still return `Option`; their signature change is change 5's.

The check (`EventHandler` / `DerivedHandler` / `accumulate`) is ignored by this design: it is increasingly at odds with it and is expected to be retired rather than migrated. Same-trigger-at-two-depths needs no static ban; the claim resolves it, deepest first.

## The demo: `A → B`, everything the user writes

The demo is the acceptance surface, not a change of its own: the handlers and generated bodies are change 5's target, and the tree as declared compiles once change 5 lands (`#[pre_post]` parses there); the full walks are change 6.

`A` is the root and holds the layer `B`. `B` arms a return-home timer; every key while `B` is up pushes the deadline out; a leave must cancel the timer, because the OS timer outlives the active path and `Drop` cannot emit the cancel. One post owns the whole deadline story by matching the state.

```rust
type APath<'a> = &'a mut A;
type BPath<'a> = PathMut<B, APath<'a>>;

#[derive(Clone, Copy)]
struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        Self(1)
    }
}

struct TimerGuard {
    id: TimerId,
}

// #[post] / #[pre_post] are new (change 5); the other attributes are today's
// derive surface.
// The deadline post keys on what the descent did, so it is scheduled before
// the bind (source order).
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))] // opt_0
#[bind(KeyEsc => flash)]                                        // opt_1
struct A {
    #[resolve_into]
    b: B,
}

#[derive(Bind)]
#[node(parent = APath)]
#[binds(M)]
#[bind(KeyH => go_home)]
struct B {
    return_home: TimerGuard,
}

enum DemoEffect {
    ScheduleTimer(TimerId),
    CancelTimer(TimerId),
    FlashOverlay,
}

/// The Bindings marker: `M: Bindings<Output = Vec<DemoEffect>>`.
struct M;
```

The handlers, all user-written:

```rust
/// B's bind: go home.
fn go_home<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, BPath<'x>>,
) -> (Vec<DemoEffect>, Completed<BPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(b) => (vec![], b.into_parent().complete()), // Up(a)
        MaybeInvalidated::Invalidated(c) => (vec![], c),
    }
}

/// A's bind.
fn flash<'x>(
    _ev: &KeyEvent,
    _snap: (),
    st: AscendState<'_, APath<'x>>,
) -> (Vec<DemoEffect>, Completed<APath<'x>>) {
    (vec![DemoEffect::FlashOverlay], st.complete())
}

/// A's pre: runs before descending into B, while the old timer id is live.
/// A pre takes `&Self::Path` at every depth (field access auto-derefs through
/// the root's `&&mut A` as through `&PathMut`).
fn snap_return_home(_ev: &KeyEvent, a: &APath<'_>) -> TimerId {
    a.b.return_home.id
}

/// A's post: the whole return-home deadline. B on the active path → push the
/// deadline out. Invalidated → the snap is all that is left of the timer.
fn return_home_deadline<'x>(
    _ev: &KeyEvent,
    snapped: TimerId,
    st: AscendState<'_, APath<'x>>,
) -> (Vec<DemoEffect>, Completed<APath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(a) => {
            let fresh = TimerId::fresh();
            a.b.return_home = TimerGuard { id: fresh };
            (
                vec![DemoEffect::CancelTimer(snapped), DemoEffect::ScheduleTimer(fresh)],
                a.complete(),
            )
        }
        MaybeInvalidated::Invalidated(c) => (vec![DemoEffect::CancelTimer(snapped)], c),
    }
}
```

`Stop` never appears in user code. Every arm returns a `Completed`: staying put is `st.complete()` or `path.complete()`, leaving is `into_parent()` then `.complete()`, and the already-invalidated arm forwards `c`.

## Generated: B (target, leaf)

```rust
impl Dispatch<M> for B {
    fn dispatch<'a, 'c>(
        path: BPath<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<BPath<'a>>
    where
        Self: 'a,
    {
        let opt_0 = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyH;
            if EventTrigger::is_matching(&trigger, ev) {
                Some((ev, (|_, _| ())(ev, &path)))
            } else {
                None
            }
        } else {
            None
        };

        let mut state = MaybeInvalidated::NotInvalidated(path);

        if let Some((ev, snap)) = opt_0 {
            let (e, completed) = (::bind::exclusive(go_home))(ev, snap, AscendState::new(state, claim.reborrow()));
            effs.extend(e);
            state = completed.to_maybe_invalidated();
        }

        state.complete()
    }
}
```

## Generated: A (target)

```rust
impl Dispatch<M> for A
where
    B: Dispatch<M>,
{
    fn dispatch<'a, 'c>(
        path: &'a mut A,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Completed<&'a mut A>
    where
        Self: 'a,
    {
        // Snapped before descent: the schedule is final.
        let opt_0 = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some((ev, (snap_return_home)(ev, &path)))
            } else {
                None
            }
        } else {
            None
        };

        let opt_1 = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyEsc;
            if EventTrigger::is_matching(&trigger, ev) {
                Some((ev, (|_, _| ())(ev, &path)))
            } else {
                None
            }
        } else {
            None
        };

        let b_path = laserbeam::PathMut::from_fn(path, |a| &mut a.b, |a| &a.b);

        let mut state = B::dispatch(b_path, event, effs, claim).into_inner().to_maybe_invalidated();

        if let Some((ev, snapped)) = opt_0 {
            let (e, completed) = return_home_deadline(ev, snapped, AscendState::new(state, claim.reborrow()));
            effs.extend(e);
            state = completed.to_maybe_invalidated();
        }

        if let Some((ev, snap)) = opt_1 {
            let (e, completed) = (::bind::exclusive(flash))(ev, snap, AscendState::new(state, claim.reborrow()));
            effs.extend(e);
            state = completed.to_maybe_invalidated();
        }

        state.complete()
    }
}
```

The same body serves every node: only the `state` construction differs (leaf: `NotInvalidated(path)`; parent: the child call chained through `to_maybe_invalidated`), and that difference is one expression, not a shape.

## bind and bind_macro (before / after)

What changes where: the free `dispatch` return in `bind/src/lib.rs` (change 2, landed); opts emission in `dispatch_impl` (change 3, landed); the trait signatures, `dispatch_impl`, `dispatch_body` (route folds included), the place `dispatch_into_parent_impl` deletion, `up =` parsing, `#[post]` / `#[pre_post]` registration and parsing, `derived_node_impl`, and `derived_enum_node_impl` (change 5, with `derived-levels.md`); the demo tree and full walks (change 6).

### Change 2 (landed) — free `dispatch`: the claim alone means handled

Before:

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

Consumers (mercury's loop, `SimpleRunner`, tests) perform the effects unconditionally and use the bool for pass-through.

### Change 3 (landed) — opts before descent, source order

Before, each check builds its trigger inline, after the recursion, inside `dispatch_impl`'s `checks`:

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

After: one opt local per scheduled attribute, emitted before the descent, numbered in source order across all kinds; the checks consume the opts but keep their current firing form, so this change is behavior-visible only in when triggers and pres read state (pre-descent, which is the point):

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

### Change 5 — the linear `Completed` body

The trait signatures in `bind/src/lib.rs`, before (on disk):

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> Option<Self::Path<'a>>
    where
        Self: 'a;
}

pub trait DispatchIntoParent<M: Bindings>: HasParent + Sized {
    fn dispatch_into_parent(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'_>,
    ) -> Option<Self::Parent>;
}
```

After:

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'c>,
    ) -> ::laserbeam::Completed<Self::Path<'a>>
    where
        Self: 'a,
        Self::Path<'a>: ::laserbeam::HasStop;
}

```

`DispatchIntoParent` is deleted, replaced by `DispatchIntoPlace` (`derived-levels.md`).

`dispatch_impl`, emitted signature and shape, before:

```rust
fn dispatch<'a, 'c>(
    #binding: <Self as ::bind::Place>::Path<'a>,
    event: &<#marker as ::bind::Bindings>::Event,
    effs: &mut <#marker as ::bind::Bindings>::Output,
    claim: &mut ::bind::Claim<'c>,
) -> ::core::option::Option<<Self as ::bind::Place>::Path<'a>>
where
    Self: 'a,
{
    #recurse
    #(#checks)*
    ::core::option::Option::Some(path)
}
```

After:

```rust
fn dispatch<'a, 'c>(
    #binding: <Self as ::bind::Place>::Path<'a>,
    event: &<#marker as ::bind::Bindings>::Event,
    effs: &mut <#marker as ::bind::Bindings>::Output,
    claim: &mut ::bind::Claim<'c>,
) -> ::laserbeam::Completed<<Self as ::bind::Place>::Path<'a>>
where
    Self: 'a,
    <Self as ::bind::Place>::Path<'a>: ::laserbeam::HasStop,
{
    #(#opts)*
    let mut state = #state;
    #(#scheduled)*
    state.complete()
}
```

`#state` for a leaf: `::laserbeam::MaybeInvalidated::NotInvalidated(path)`. For a node with a child, from `dispatch_body` — before (the `?` rides the `Option`):

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim)?;
path = #recover;
```

After (the root/non-root split lives in laserbeam's two `Stop::to_maybe_invalidated` impls, not here; `Edge::recover_parent` is subsumed by the method's `into_parent`; the enum-child case emits the same expression per variant arm, the whole match being `#state`):

```rust
::laserbeam::Completed::into_inner(
    <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim),
)
.to_maybe_invalidated()
```

For a route edge (`#[resolve_into(parent = .., up = ..)]`), `#state` is instead the fold in "Route enums", recovering the descent's variant.

One scheduled block per item, kind-blind; `#rhs` is `::bind::exclusive(#tokens)` for `#[bind]` and the raw tokens otherwise, and the macro looks inside neither:

```rust
if let ::core::option::Option::Some((ev, snap)) = opt_N {
    let (e, completed) = (#rhs)(ev, snap, ::bind::AscendState::new(state, ::bind::Claim::reborrow(claim)));
    ::core::iter::Extend::extend(effs, e);
    state = ::laserbeam::Completed::to_maybe_invalidated(completed);
}
```

`dispatch_into_parent_impl`, before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, effs, claim) {
    ::core::option::Option::None => ::core::option::Option::None,
    ::core::option::Option::Some(path) => {
        ::core::option::Option::Some(::bind::HasParent::into_parent(path))
    }
}
```

After: not emitted, for any place. The impl has no caller (the derived-child descent calls `Node` impls), and its `Self::Parent: HasStop` bound is unsatisfiable for route-parented nodes.

`derived_node_impl` / `derived_enum_node_impl` migrate per `derived-levels.md`, in this same change.

Handler migration, in the same workspace change: every bind handler in mercury and the bind tests goes from `(ev, Node<P, ()>) -> impl IntoIterator<Item = E>` to `(ev, _snap: (), AscendState<P>) -> (Vec<E>, Completed<P>)`, with `st.complete()` where the body stays put.

### Change 5 (parsing) — `#[post]` / `#[pre_post]`

Registration, before / after:

```rust
#[proc_macro_derive(
    Bind,
    attributes(binds, bind, resolve_into, derived_child, derived_node, node)
)]
// →
#[proc_macro_derive(
    Bind,
    attributes(binds, bind, post, pre_post, resolve_into, derived_child, derived_node, node)
)]
```

Like `#[bind]` today, every kind takes multiple comma-separated pairs (`#[post(a => b, c => d)]`), parsed as a `Punctuated` list and scheduled in source order within the attribute. Parsing, beside `Binding` (whose `Parse` the plain `#[post]` form reuses), plus collectors `posts(attrs)` / `pre_posts(attrs)` mirroring `binds()`:

```rust
/// One `trigger => (pre, post)` pair from `#[pre_post(..)]`.
struct PrePost {
    trigger: Expr,
    pre: Expr,
    post: Expr,
}

impl syn::parse::Parse for PrePost {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let trigger = input.parse()?;
        input.parse::<Token![=>]>()?;
        let content;
        syn::parenthesized!(content in input);
        let pre = content.parse()?;
        content.parse::<Token![,]>()?;
        let post = content.parse()?;
        Ok(Self { trigger, pre, post })
    }
}
```

All three attribute kinds feed one scheduled list in source order; the differences are confined to parse time (which tokens fill `#pre`, written or synthesized, and whether the rhs gets the `exclusive` wrap). The check is ignored (see Landed baseline), so nothing here touches `claimed_triggers` or `accumulate`.

### Derived levels

The design is `derived-levels.md`: ascent flattens to the place path beneath the `Node` chain (`HasPlace`), `DispatchIntoParent` becomes `DispatchIntoPlace`, and derived data reaches handlers through pres. Change 5 implements it.

## Walks

### KeyH: B goes home

```text
B:  exclusive(go_home): claim won → Up(a); fold → Invalidated
    state.complete() → the stored Completed
A:  state = Invalidated(root.complete())
    return_home_deadline (scheduled first): Invalidated → [CancelTimer(snapped)]
    fold of its returned c: root Completed → NotInvalidated
    exclusive(flash): claim already taken → completes where it stands; fold
    state.complete() → complete(path)
```

### Any other key: B stays

```text
B:  fallthrough → Here(b)
A:  state = NotInvalidated(b.into_parent())
    return_home_deadline: NotInvalidated → [CancelTimer(old), ScheduleTimer(fresh)]
    KeyEsc only: exclusive(flash) fires → [FlashOverlay]
    state.complete() → complete(path)
```

Posts run whether or not anything claimed: they are scheduled by their trigger, not by the claim.

## Rules

1. No stubs.
2. Between nodes: `Completed` / `Stop`, `Here` / `Up`. Inside a node: the state, built once via `to_maybe_invalidated`, handed to each scheduled item in a fresh `AscendState` (claim reborrowed), re-derived by the generated code from each item's returned `Completed`, completed with `state.complete()`. `Stop` never appears in user code.
3. Every place dispatch returns `Completed<Self::Path>`; no ascent associated type. Derived levels: `derived-levels.md`.
4. Opts are snapped before descent, one per scheduled attribute, in source order. The schedule is final; every scheduled item runs, and its body decides what each state branch means.
5. Every handler is `(ev, snap, AscendState<P>) -> (Vec<E>, Completed<P>)` (`snap = ()` under the synthesized pre) and returns the call to `.complete()`: staying put is `st.complete()`, leaving is `into_parent()` then `.complete()`, the invalidated arm forwards `c`. `exclusive` is shape-preserving and means not claimed: the claim gate and nothing else. The state a handler receives reflects the item before it, so a post keyed on the descent's outcome is scheduled before any bind.
6. The claim lives inside `AscendState`; only binds claim. The check is ignored by this design.
7. Generated code spells laserbeam and bind items fully qualified; handwritten handlers `use laserbeam::{Complete, MaybeInvalidated};` and `use bind::AscendState;`.
8. Route enums: the consumer hand-writes the route enum, the Up enum, and the `Above` impl; `#[resolve_into(parent = .., up = ..)]` informs the macro; the generated fold recovers the descent's variant and `unreachable!()`s the others; a leave wraps one `Up` level by hand. No place emits an into-parent dispatch impl (`derived-levels.md`).

## Tests

Unit tests on the laserbeam items land with change 1; the rest land with the change that makes them expressible (the full A/B walks: change 6).

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- a three-level tree: `Invalidated` forwards through `state.complete()` unchanged
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts run without a claim, and on both branches of `MaybeInvalidated`
- pre snap reads pre-descent state even when the descent mutates it
- a leaving item flips the state to `Invalidated` for later items; the fold
  after a staying item re-derives `NotInvalidated`
- multi-parent: `t` fires under each route and the fold re-establishes
  `NotInvalidated`; `home` (`title_home`) leaves through each route and the
  fold forwards `Invalidated` (change 5)
- the `Completed<TitlePath>` shape pin (change 4)

## Ordered changes

One agent, strictly in order: implement each change, get the workspace green, commit, then start the next. The numbering already satisfies every prerequisite. The code deltas live in the labeled additions sections and "bind and bind_macro (before / after)".

### 0 — the standalone `DispatchIntoParent` rename (landed, fa2f155)

### 1 — laserbeam: `MaybeInvalidated` (+ `complete`), the `to_maybe_invalidated` conversions, `From<&mut R> for Completed<&mut R>` (landed, 74e8077)

### 2 — bind: `AscendState`, `exclusive`, `Claim::reborrow`; free `dispatch` returns `(Vec<E>, bool)` (landed, 7b19f86)

### 3 — bind_macro: opts before descent, source order, synthesized `|_, _| ()` pre (landed, 2f6c083)

### 4 — route enums: `TitleParentUp` + `Above` beside `TitleParent`; the trybuild negative deleted

Pure additions plus the flip ("Route enums"); the shape pin lands here. Nothing else consumes the new types until change 5, and the attribute changes (`up =`, `route`), the `home` bind, and `title_home` wait for change 5's handler shape.

### 5 — bind_macro: trait signatures, the linear `Completed` body, scheduled blocks, the place `dispatch_into_parent_impl` deleted, route folds, `up =` parsing, `#[post]` / `#[pre_post]` parsing, derived levels (`derived-levels.md`); handler migration in mercury and tests

The one big change; it cannot split further because the handler signature change is workspace-global, and the migrated derived tests read data through pres, which is why the parsing is here rather than in change 6. `derived-levels.md`'s change 1 (`HasPlace` + `DispatchIntoPlace`, pure additions) lands immediately before as its own commit. Blocked until `derived-levels.md` is settled.

### 6 — the demo tree and full walks live (the `#[pre_post]` on `A`)
