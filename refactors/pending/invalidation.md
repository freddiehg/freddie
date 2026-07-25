# Invalidation: descent schedules, ascent runs posts

Not done. Prefactor `path-peel-complete.md`: landed. Invalidation change "Claim + effs sink, drop ControlFlow" (22e5580): landed. Every "before" below is the code on disk after that commit. Every new type and generated shape below is compile-checked in the design scratch against the real laserbeam.

Descent schedules which posts run. That set is final. Ascent runs every scheduled post.

## Model

Every node's dispatch returns `Completed<Self::Path<'a>>`, root included. Between nodes, the protocol is the child's `Completed` and its `Stop` arms. Inside a node, dispatch is one linear, kind-blind body:

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

## laserbeam additions

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

One conversion is added; generated `DispatchIntoParent` code normalizes an Up payload, bare root path or `Completed`, behind one `Into`:

```rust
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

## bind additions

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

## Landed baseline

`bind/src/lib.rs` already holds `Claim` (which gains only `reborrow`, in change 2), the final `Dispatch` and `DispatchIntoParent` signatures, and the free `dispatch` (whose return changes in change 2: the `!effs.is_empty()` half of its condition was never asked for and is wrong, since posts emit effects on unclaimed keys).

```rust
/// One exclusive bind handler per dispatch: the first to `try_take` wins.
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

impl<'c> Claim<'c> {
    pub fn new(slot: &'c mut Option<()>) -> Self {
        Self { slot }
    }

    pub const fn is_taken(&self) -> bool {
        self.slot.is_some()
    }

    /// `Some(())`: you won the claim. `None`: someone already has it.
    pub const fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }
}

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

pub trait DispatchIntoParent<M: Bindings>: HasParent + Sized
where
    Self::Parent: ::laserbeam::HasStop,
{
    fn dispatch_into_parent(
        self,
        event: &M::Event,
        effs: &mut M::Output,
        claim: &mut Claim<'_>,
    ) -> ::laserbeam::Completed<Self::Parent>;
}

pub fn dispatch<'a, M, N, E>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<E>>
where
    M: Bindings<Output = Vec<E>>,
    N: Dispatch<M> + 'a,
    N::Path<'a>: ::laserbeam::HasStop,
{
    let mut effs: Vec<E> = Vec::new();
    let mut claim_slot = None;
    let mut claim = Claim::new(&mut claim_slot);
    let _completed = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claim);
    if claim.is_taken() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

The check (`EventHandler` / `DerivedHandler` / `accumulate`) is untouched by everything below.

## The demo: `A → B`, everything the user writes

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

// #[post] / #[pre_post] are new (change 4); the other attributes are today's
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

What changes where: the free `dispatch` return in `bind/src/lib.rs` (change 2); opts emission in `dispatch_impl` (change 3); `dispatch_impl`, `dispatch_body`, `dispatch_into_parent_impl`, `derived_node_impl`, and `derived_enum_node_impl` (change 4); attribute registration and parsing (change 5).

### Change 2 — free `dispatch`: the claim alone means handled

Before (landed):

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

### Change 3 — opts before descent, source order

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

### Change 4 — the linear `Completed` body

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

After (`Into::into` covers both parent shapes via the laserbeam `From` impl):

```rust
match ::laserbeam::Completed::into_inner(
    <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, effs, claim),
) {
    ::laserbeam::Stop::Here(path) => ::laserbeam::Complete::complete(
        ::bind::HasParent::into_parent(path),
    ),
    ::laserbeam::Stop::Up(rest) => ::core::convert::Into::into(rest),
}
```

`derived_node_impl` / `derived_enum_node_impl` get the interim migration (their full story is change 6). The fallthrough, before / after:

```rust
::core::option::Option::Some(::bind::HasParent::into_parent(node))
// →
::laserbeam::Complete::complete(::bind::HasParent::into_parent(node))
```

and their checks keep the old firing form but end with that same `Complete::complete` instead of `return None`.

Handler migration, in the same workspace change: every bind handler in mercury and the bind tests goes from `(ev, Node<P, ()>) -> impl IntoIterator<Item = E>` to `(ev, _snap: (), AscendState<P>) -> (Vec<E>, Completed<P>)`, with `st.complete()` where the body stays put.

### Change 5 — `#[post]` / `#[pre_post]` parsing

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

All three attribute kinds feed one scheduled list in source order; the differences are confined to parse time (which tokens fill `#pre`, written or synthesized, and whether the rhs gets the `exclusive` wrap). `claimed_triggers` does not change: only `#[bind]` triggers claim, so posts are exempt from the duplicate-trigger check.

### Change 6 — derived levels

Derived-level binds migrate to the scheduled shape over `AscendState<Self::Parent>`, and posts across derived-child edges get a story; `DispatchIntoParent`'s `Here` collapse currently hides child-alive from the caller.

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
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent, one per scheduled attribute, in source order. The schedule is final; every scheduled item runs, and its body decides what each state branch means.
5. Every handler is `(ev, snap, AscendState<P>) -> (Vec<E>, Completed<P>)` (`snap = ()` under the synthesized pre) and returns the call to `.complete()`: staying put is `st.complete()`, leaving is `into_parent()` then `.complete()`, the invalidated arm forwards `c`. `exclusive` is shape-preserving and means not claimed: the claim gate and nothing else. The state a handler receives reflects the item before it, so a post keyed on the descent's outcome is scheduled before any bind.
6. The claim lives inside `AscendState`; only binds claim, so posts are exempt from the duplicate-trigger check.
7. Generated code spells laserbeam and bind items fully qualified; handwritten handlers `use laserbeam::{Complete, MaybeInvalidated};` and `use bind::AscendState;`.

## Tests

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- a three-level tree: `Invalidated` forwards through `state.complete()` unchanged
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts run without a claim, and on both branches of `MaybeInvalidated`
- pre snap reads pre-descent state even when the descent mutates it
- a leaving item flips the state to `Invalidated` for later items; the fold
  after a staying item re-derives `NotInvalidated`

## Open questions

1. The duplicate-trigger check still rejects the same trigger bound at two depths of the active path, while the claim makes that combination well-defined (deepest claimant wins). Keep the ban, or relax it to same-node duplicates only? The demo dodges by using distinct keys.
2. `#[post]` and `#[pre_post]` are now the same thing modulo the synthesized pre. Keep both spellings, or collapse to `#[post]` with an optional `(pre, f)` rhs?
3. `TimerId::fresh()` is demo filler; real ids mint from root state per `timer-ids-on-root.md`.

## Ordered changes

Each change compiles and passes tests against only its predecessors; per-change landability is noted. The code deltas live in "bind and bind_macro (before / after)" and the additions sections.

### 0 — the standalone `DispatchIntoParent` rename (section above; in flight)

### 1 — laserbeam: `MaybeInvalidated` (+ `complete`), the `to_maybe_invalidated` conversions, `From<&mut R> for Completed<&mut R>`

Pure additions with unit tests; nothing consumes them yet.

### 2 — bind: `AscendState`, `exclusive`, `Claim::reborrow`; free `dispatch` returns `(Vec<E>, bool)`

Additions plus one signature change; consumers of the free `dispatch` (mercury's loop, `SimpleRunner`, tests) migrate mechanically in the same change.

### 3 — bind_macro: opts before descent, source order, synthesized `|_, _| ()` pre

Behavior change confined to when triggers and pres read state; checks keep their firing form, so it lands alone.

### 4 — bind_macro: the linear `Completed` body, scheduled blocks, `dispatch_into_parent_impl`, derived interim; handler migration in mercury and tests

The one big change; it cannot split further because the handler signature change is workspace-global.

### 5 — `#[post]` / `#[pre_post]`: registration, parsing, collectors

Additive; until it lands, only `#[bind]` items populate the schedule.

### 6 — derived levels: binds to the scheduled shape; derived-edge posts
