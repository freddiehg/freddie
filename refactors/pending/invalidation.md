# Invalidation: descent schedules, ascent runs posts

Not done. Prefactor `path-peel-complete.md`: landed. Invalidation change "Claim + effs sink, drop ControlFlow" (22e5580): landed. Every "before" below is the code on disk after that commit. Every new type and generated shape below is compile-checked in the design scratch against the real laserbeam.

Descent schedules which posts run. That set is final. Ascent runs every scheduled post.

## Model

Every node's dispatch returns `Completed<Self::Path<'a>>`, root included. Between nodes, the protocol is the child's `Completed` and its `Stop` arms. Inside a node, dispatch is one linear, kind-blind body:

```text
opts        one local per scheduled attribute, source order, snapped before descent
descend     child dispatch → .into_inner().to_maybe_invalidated() → AscendState
scheduled   one identical block per item: call, extend effs, rebind the state
finish      st.finish()
```

Handlers take the event payload and the ascend state. A `#[post]` rhs is taken raw, in the one scheduled shape:

```rust
// the scheduled shape: every #[post] rhs, verbatim
FnOnce(Payload, AscendState<'a, 'c, P>) -> (Vec<E>, AscendState<'a, 'c, P>)

// a bind handler, lifted into it by exclusive
FnOnce(&Ev, AscendState<'a, 'c, P>) -> (Vec<E>, Completed<P>)
```

The state evolves through the schedule. With `#[post(a => b, c => d)]`, `b` can receive `NotInvalidated`, leave (`into_parent().complete()`, placing the result in `Invalidated`), and `d` then receives the invalidated state. Every scheduled item runs; a leave is data in the state, not control flow, and nothing early-returns.

`#[bind(X => foo)]` desugars to `#[post(X => exclusive(foo))]`, token wrapping and nothing more; the macro never looks inside any rhs. `exclusive` is how the framework controls whether a bind handler is called: `foo` runs iff the state is `NotInvalidated` and the claim is won, and its returned `Completed` is folded back into the state (`reopen`): stayed is `NotInvalidated` again, left is `Invalidated`. A gated state passes through untouched (untouched matters: round-tripping it through a `Completed` would erase `ChildInvalidated`).

"Invalidated" means off the active path: focus left it. Whether state was also replaced is the handler's business (an enum layer usually swaps; a struct field persists).

## laserbeam additions

`MaybeInvalidated` is active-path semantics, meaningful in a tree with no bindings, so it lives beside `Stop`:

```rust
/// After a child's leave: this path's situation on the active path.
pub enum MaybeInvalidated<P: HasStop> {
    /// The child kept focus; this node's path, recovered.
    NotInvalidated(P),
    /// The leave stopped at this node; the child is off the active path.
    ChildInvalidated(P),
    /// This node is off the active path too. Carries the node's own completed
    /// leave, ready to forward, built where the payload existed.
    Invalidated(Completed<P>),
}

impl<P: HasStop> MaybeInvalidated<P>
where
    P: Complete<P>,
{
    /// Where dispatch stands: total, no dead arms.
    pub fn finish(self) -> Completed<P> {
        match self {
            Self::NotInvalidated(path) | Self::ChildInvalidated(path) => path.complete(),
            Self::Invalidated(completed) => completed,
        }
    }
}

/// A child of the root: the Up payload is the bare root path.
impl<'a, N, R> Stop<PathMut<N, &'a mut R>, &'a mut R> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<&'a mut R> {
        match self {
            Stop::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Stop::Up(root) => MaybeInvalidated::ChildInvalidated(root),
        }
    }
}

/// A child of a non-root: the Up payload is this node's own Completed.
impl<N, N2, Q: Above> Stop<PathMut<N, PathMut<N2, Q>>, Completed<PathMut<N2, Q>>> {
    pub fn to_maybe_invalidated(self) -> MaybeInvalidated<PathMut<N2, Q>> {
        match self {
            Stop::Here(child) => MaybeInvalidated::NotInvalidated(child.into_parent()),
            Stop::Up(rest) => match rest.into_inner() {
                Stop::Here(path) => MaybeInvalidated::ChildInvalidated(path),
                Stop::Up(above) => MaybeInvalidated::Invalidated(Completed::up(above)),
            },
        }
    }
}
```

`HasStop` (landed) gains one associated fn, implemented by its two existing impls; it is what folds a fired handler's `Completed` back into the state:

```rust
pub trait HasStop: Sized {
    type Stop;
    /// Re-open a completed leave from this path into the active-path state.
    fn reopen(completed: Completed<Self>) -> MaybeInvalidated<Self>;
}

impl<N, P: Above> HasStop for PathMut<N, P> {
    type Stop = Stop<PathMut<N, P>, P::Up>;
    fn reopen(completed: Completed<Self>) -> MaybeInvalidated<Self> {
        match completed.into_inner() {
            Stop::Here(path) => MaybeInvalidated::NotInvalidated(path),
            Stop::Up(rest) => MaybeInvalidated::Invalidated(Completed::up(rest)),
        }
    }
}

impl<'a, R> HasStop for &'a mut R {
    type Stop = &'a mut R;
    fn reopen(completed: Completed<Self>) -> MaybeInvalidated<Self> {
        MaybeInvalidated::NotInvalidated(completed.into_inner())
    }
}
```

And one conversion so `Descend` normalizes an Up payload (bare root path, or already a `Completed`) with one `Into`:

```rust
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

## bind additions

```rust
/// What every scheduled handler receives beside the event: the claim, and
/// where this dispatch stands after its child returned.
pub struct AscendState<'a, 'c, P: ::laserbeam::HasStop> {
    claim: &'a mut Claim<'c>,
    pub state: ::laserbeam::MaybeInvalidated<P>,
}

impl<'a, 'c, P: ::laserbeam::HasStop> AscendState<'a, 'c, P> {
    pub fn new(state: ::laserbeam::MaybeInvalidated<P>, claim: &'a mut Claim<'c>) -> Self {
        Self { claim, state }
    }

    pub fn claim(&mut self) -> Option<()> {
        self.claim.try_take()
    }

    /// The bind gate: the path and the claim's remainder, iff nothing is
    /// invalidated and the claim is won. `Err` hands everything back untouched.
    pub fn exclusive(self) -> Result<(P, &'a mut Claim<'c>), Self> {
        let Self { claim, state } = self;
        match state {
            ::laserbeam::MaybeInvalidated::NotInvalidated(path) => match claim.try_take() {
                Some(()) => Ok((path, claim)),
                None => Err(Self {
                    claim,
                    state: ::laserbeam::MaybeInvalidated::NotInvalidated(path),
                }),
            },
            state => Err(Self { claim, state }),
        }
    }

    pub fn finish(self) -> ::laserbeam::Completed<P>
    where
        P: ::laserbeam::Complete<P>,
    {
        self.state.finish()
    }
}

/// Lifts a bind handler into the scheduled shape. The framework decides
/// whether the handler runs; a fired handler's Completed is folded back into
/// the state; a gated state passes through untouched.
pub fn exclusive<Ev, P, E, H>(
    handler: H,
) -> impl for<'a, 'c> FnOnce(&Ev, AscendState<'a, 'c, P>) -> (Vec<E>, AscendState<'a, 'c, P>)
where
    P: ::laserbeam::HasStop,
    H: for<'a, 'c> FnOnce(&Ev, AscendState<'a, 'c, P>) -> (Vec<E>, ::laserbeam::Completed<P>),
{
    move |ev, st| match st.exclusive() {
        Ok((path, claim)) => {
            let (e, completed) = handler(
                ev,
                AscendState::new(::laserbeam::MaybeInvalidated::NotInvalidated(path), &mut *claim),
            );
            (e, AscendState::new(P::reopen(completed), claim))
        }
        Err(st) => (Vec::new(), st),
    }
}

```

## Landed baseline (no further change)

`bind/src/lib.rs` already holds `Claim` (`try_take` stays if/else because it is `const fn` and `Option::replace` is not const), the final `Dispatch` and `Descend` signatures, and the final free `dispatch`:

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

pub trait Descend<M: Bindings>: HasParent + Sized
where
    Self::Parent: ::laserbeam::HasStop,
{
    fn dispatch(
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

// #[bind], #[node], #[binds], #[resolve_into] are today's derive surface.
// #[post] / #[pre_post] are new in change 4. Opts are numbered in source
// order; #[bind(X => foo)] is #[post(X => exclusive(foo))].
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(KeyEsc => flash)]                                        // opt_0
#[pre_post(AnyKey => (snap_return_home, return_home_deadline))] // opt_1
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

The handlers, all user-written; every one takes the event payload and the ascend state:

```rust
/// B's bind: go home. Everything below root leaves the active path.
fn go_home<'a, 'c, 'x>(
    _ev: &KeyEvent,
    st: AscendState<'a, 'c, BPath<'x>>,
) -> (Vec<DemoEffect>, Completed<BPath<'x>>) {
    match st.state {
        MaybeInvalidated::NotInvalidated(b) => (vec![], b.into_parent().complete()), // Up(a)
        state => (vec![], state.finish()),
    }
}

/// A's bind: fires only when nothing deeper claimed the key; stays put.
fn flash<'a, 'c, 'x>(
    _ev: &KeyEvent,
    st: AscendState<'a, 'c, APath<'x>>,
) -> (Vec<DemoEffect>, Completed<APath<'x>>) {
    (vec![DemoEffect::FlashOverlay], st.finish())
}

/// A's pre: runs before descending into B, while the old timer id is live.
fn snap_return_home(_ev: &KeyEvent, a: &A) -> TimerId {
    a.b.return_home.id
}

/// A's post, in the scheduled shape raw: the whole return-home deadline, one
/// handler over the state. B on the active path → push the deadline out.
/// Anything invalidated → the snap is all that is left of the timer; cancel.
fn return_home_deadline<'a, 'c, 'x>(
    (_ev, snapped): (&KeyEvent, TimerId),
    mut st: AscendState<'a, 'c, APath<'x>>,
) -> (Vec<DemoEffect>, AscendState<'a, 'c, APath<'x>>) {
    let effects = match &mut st.state {
        MaybeInvalidated::NotInvalidated(a) => {
            let fresh = TimerId::fresh();
            a.b.return_home = TimerGuard { id: fresh };
            vec![DemoEffect::CancelTimer(snapped), DemoEffect::ScheduleTimer(fresh)]
        }
        _ => vec![DemoEffect::CancelTimer(snapped)],
    };
    (effects, st)
}
```

`Stop` never appears in user code. A bind handler that stays put is `st.finish()`; one that leaves peels and completes; the arms it cannot be called in (the framework gates it to `NotInvalidated`) fall through to `state.finish()`.

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
        let opt_0: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyH;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        let mut st = AscendState::new(MaybeInvalidated::NotInvalidated(path), claim);

        if let Some(ev) = opt_0 {
            let (e, next) = (::bind::exclusive(go_home))(ev, st);
            effs.extend(e);
            st = next;
        }

        st.finish()
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
        // Opts, source order, snapped before descent: the schedule is final.
        let opt_0: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = KeyEsc;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        let opt_1: Option<(&KeyEvent, TimerId)> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some((ev, snap_return_home(ev, path)))
            } else {
                None
            }
        } else {
            None
        };

        let b_path = laserbeam::PathMut::from_fn(path, |a| &mut a.b, |a| &a.b);

        let mut st = AscendState::new(
            B::dispatch(b_path, event, effs, claim).into_inner().to_maybe_invalidated(),
            claim,
        );

        if let Some(ev) = opt_0 {
            let (e, next) = (::bind::exclusive(flash))(ev, st);
            effs.extend(e);
            st = next;
        }

        if let Some(payload) = opt_1 {
            let (e, next) = return_home_deadline(payload, st);
            effs.extend(e);
            st = next;
        }

        st.finish()
    }
}
```

The same body serves every node: only the `st` construction differs (leaf: `NotInvalidated(path)`; parent: the child call chained through `to_maybe_invalidated`), and that difference is one expression, not a shape.

## bind_macro (before / after)

### Change 1 — emit `Completed`; the linear body

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
    let mut st = ::bind::AscendState::new(#state, claim);
    #(#scheduled)*
    st.finish()
}
```

`#state` for a leaf: `::laserbeam::MaybeInvalidated::NotInvalidated(path)`. For a node with a child, from `dispatch_body` — before (the `?` rides the `Option`):

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim)?;
path = #recover;
```

After (the root/non-root split lives in laserbeam's two `to_maybe_invalidated` impls, not here; `Edge::recover_parent` is subsumed by the method's `into_parent`; the enum-child case emits the same per variant arm):

```rust
::laserbeam::Completed::into_inner(
    <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim),
)
.to_maybe_invalidated()
```

`descend_impl`, before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, effs, claim) {
    ::core::option::Option::None => ::core::option::Option::None,
    ::core::option::Option::Some(path) => {
        ::core::option::Option::Some(::bind::HasParent::into_parent(path))
    }
}
```

After (`Into::into` covers both parent shapes via the laserbeam `From` impl; `derived_node_impl`'s fallthrough becomes `::laserbeam::Complete::complete(::bind::HasParent::into_parent(node))` the same way):

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

### Change 2 — opts before descent, source order

Before: each check builds its trigger inline, after the recursion. After: one opt local per scheduled attribute, emitted before the descent, numbered in source order; a `#[pre_post]` opt runs its pre here, which is the point, the snap reads pre-descent state:

```rust
let opt_N = match ::core::convert::TryFrom::try_from(event) {
    ::core::result::Result::Ok(ev) => {
        let trigger = #trigger;
        if ::bind::EventTrigger::is_matching(&trigger, ev) {
            ::core::option::Option::Some(#payload) // ev, or (ev, #pre(ev, &path))
        } else {
            ::core::option::Option::None
        }
    }
    ::core::result::Result::Err(_) => ::core::option::Option::None,
};
```

### Change 3 — `AscendState` threading; one scheduled block

bind gains `AscendState`, `exclusive`, `post`; laserbeam gains `MaybeInvalidated` (+ `finish`), `to_maybe_invalidated`, and `HasStop::reopen` (code above). The check emission, before: the change-1 `*effs = collect(..)` form. After, one kind-blind block per scheduled item; `#rhs` is the attribute's rhs tokens, taken raw for `#[post]`/`#[pre_post]` and wrapped as `::bind::exclusive(#tokens)` for `#[bind]`, and the macro never looks inside:

```rust
if let ::core::option::Option::Some(payload) = opt_N {
    let (e, next) = (#rhs)(payload, st);
    ::core::iter::Extend::extend(effs, e);
    st = next;
}
```

Every bind handler in mercury and the bind tests migrates: `(ev, Node<P, ()>) -> impl IntoIterator<Item = E>` becomes `(ev, AscendState<P>) -> (Vec<E>, Completed<P>)`, with `st.finish()` where the body stays put.

### Change 4 — `#[post]` / `#[pre_post]` parsing

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

Like `#[bind]` today, every kind takes multiple comma-separated pairs (`#[post(a => b, c => d)]`), parsed as a `Punctuated` list and scheduled in source order within the attribute. Parsing, beside `Binding` (whose `Parse` the plain `#[post]` form reuses):

```rust
/// One `trigger => (pre, post)` pair from `#[pre_post(..)]`: pre runs in the
/// opt, before descent; its snap rides the opt payload into the post.
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

All three attribute kinds feed one scheduled list in source order; the differences are confined to parse time (which payload the opt captures, which wrapper the rhs tokens get). `claimed_triggers` does not change: only `#[bind]` triggers claim, so posts are exempt from the duplicate-trigger check.

### Change 5 — derived-edge posts

Posts across derived-child edges, where `Descend`'s `Here` collapse currently hides child-alive from the caller.

## Walks

### KeyH: B goes home

```text
B:  exclusive(go_home): gate passes → Up(a); reopen → Invalidated
    st.finish() → the forwarded Completed
A:  st.state = ChildInvalidated(path)
    exclusive(flash): gated (claim taken) → state untouched
    post(return_home_deadline): not NotInvalidated → [CancelTimer(snapped)]
    st.finish() → complete(path)
```

### Any other key: B stays

```text
B:  fallthrough → Here(b)
A:  st.state = NotInvalidated(b.into_parent())
    KeyEsc only: exclusive(flash) fires → [FlashOverlay]; stays → NotInvalidated
    post(return_home_deadline): NotInvalidated → [CancelTimer(old), ScheduleTimer(fresh)]
    st.finish() → complete(path)
```

Posts run whether or not anything claimed: they are scheduled by their trigger, not by the claim.

## Rules

1. No stubs.
2. Between nodes: `Completed` / `Stop`, `Here` / `Up`. Inside a node: `AscendState`, built once via `to_maybe_invalidated`, threaded through every scheduled item, finished with `st.finish()`. `Stop` never appears in user code.
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent, one per scheduled attribute, in source order. The schedule is final; every scheduled item runs, and its body decides what each `MaybeInvalidated` branch means.
5. Handlers take the event payload and the ascend state. A `#[post]` rhs is the scheduled shape, raw, and may leave by placing its completed in `Invalidated`. A bind handler returns `(effects, Completed)`; `exclusive` gates it and folds the result back (`reopen`). The state a handler receives reflects everything scheduled before it.
6. The claim lives inside `AscendState`; only binds claim, so posts are exempt from the duplicate-trigger check.
7. Generated code spells laserbeam and bind items fully qualified; handwritten handlers `use laserbeam::Complete;`.

## Tests

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- a three-level tree: `Invalidated(Completed::up(..))` forwards through
  `st.finish()` unchanged, and a gated `exclusive` preserves `ChildInvalidated`
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts run without a claim, and on every branch of `MaybeInvalidated`
- pre snap reads pre-descent state even when the descent mutates it
- a fired bind that stays is `NotInvalidated` again for later posts (reopen)

## Ordered changes

Prefactors first, each independently shippable. The macro deltas per change are in "bind_macro (before / after)".

### 1 — macro emits `Completed`: signature, linear body, `descend_impl`/`derived_node_impl`; laserbeam `From<&mut R> for Completed<&mut R>`

### 2 — opts before descent, source order

### 3 — laserbeam `MaybeInvalidated` (+ `finish`), `to_maybe_invalidated`, `HasStop::reopen`; bind `AscendState`, `exclusive`; one scheduled block per item; handler migration

### 4 — `#[post]` / `#[pre_post]`: registration, parsing, payload capture

### 5 — derived-edge posts
