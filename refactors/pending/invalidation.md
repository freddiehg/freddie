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

`MaybeInvalidated<P>` answers one question, have we destroyed the path we need:

```text
NotInvalidated(P)          no — here it is
Invalidated(Completed<P>)  yes — the completed leave, ready to forward;
                           Here inside it: the leave stopped at this path
```

Every handler, bind or post, has the same signature, the one scheduled shape:

```rust
FnOnce(Payload, AscendState<'a, 'c, P>) -> (Vec<E>, AscendState<'a, 'c, P>)
```

A handler that stays put hands the state back unchanged. A handler that leaves puts the call to `.complete()` into the state (`Invalidated(path.into_parent().complete())`). The state evolves through the schedule: with `#[post(a => b, c => d)]`, `b` can receive `NotInvalidated`, leave, and `d` then receives `Invalidated`. Every scheduled item runs; a leave is data in the state, not control flow, and nothing early-returns.

`#[bind(X => foo)]` desugars to `#[post(X => exclusive(foo))]`, token wrapping and nothing more; the macro never looks inside any rhs. `exclusive` means not claimed: it is the claim gate and nothing else, calling `foo` iff the claim is won and handing the state back untouched otherwise. The claim's win is not part of any signature, because winning the claim does not imply `NotInvalidated` (a post can leave without claiming); what each state branch means is the handler's business.

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
    pub fn finish(self) -> Completed<P> {
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

`HasStop` is unchanged from what landed. One conversion is added; generated `Descend` code normalizes an Up payload, bare root path or `Completed`, behind one `Into`:

```rust
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

## bind additions

```rust
/// What every scheduled handler receives beside the event.
pub struct AscendState<'a, 'c, P: ::laserbeam::HasStop> {
    claim: &'a mut Claim<'c>,
    pub state: ::laserbeam::MaybeInvalidated<P>,
}

impl<'a, 'c, P: ::laserbeam::HasStop> AscendState<'a, 'c, P> {
    pub fn new(state: ::laserbeam::MaybeInvalidated<P>, claim: &'a mut Claim<'c>) -> Self {
        Self { claim, state }
    }

    /// `Some(())`: you won the claim. `None`: someone already has it.
    pub fn claim(&mut self) -> Option<()> {
        self.claim.try_take()
    }

    pub fn finish(self) -> ::laserbeam::Completed<P>
    where
        P: ::laserbeam::Complete<P>,
    {
        self.state.finish()
    }
}

/// The claim gate: the handler runs iff the claim is won.
pub fn exclusive<Payload, P, E, H>(
    handler: H,
) -> impl for<'a, 'c> FnOnce(Payload, AscendState<'a, 'c, P>) -> (Vec<E>, AscendState<'a, 'c, P>)
where
    P: ::laserbeam::HasStop,
    H: for<'a, 'c> FnOnce(Payload, AscendState<'a, 'c, P>) -> (Vec<E>, AscendState<'a, 'c, P>),
{
    move |payload, mut st| match st.claim() {
        Some(()) => handler(payload, st),
        None => (Vec::new(), st),
    }
}
```

## Landed baseline (no further change)

`bind/src/lib.rs` already holds `Claim`, the final `Dispatch` and `Descend` signatures, and the final free `dispatch`. (`Descend` renames to `DispatchIntoParent`; that is its own standalone change, next section.)

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

## Standalone rename: `Descend::dispatch` → `DispatchIntoParent::dispatch_into_parent`

Shippable immediately against the landed baseline; pure rename, no signature or behavior change. The method consumes a child-typed value (a place path or a derived `Node`), dispatches the event at that level, and returns `Completed<Self::Parent>`; the name follows the `into_parent` / `into_ancestor` / `into_inner` convention of a consuming step that names its output, and the trait matches its method as `Complete::complete` does.

- `bind/src/lib.rs`: rename `pub trait Descend<M: Bindings>` → `pub trait DispatchIntoParent<M: Bindings>` and its method `dispatch` → `dispatch_into_parent`. Bounds, params, and return type stay byte-identical.
- `bind/src/lib.rs`: rewrite the trait's doc comment around the capability: consumes a child-typed value, dispatches at that level, surfaces at the parent; it exists because a derived-child caller cannot name the child's type, so it calls in method position and inference finds the impl. Update every other doc-comment mention of `Descend` in the crate (crate header, `Place`, `Node`, `HasParent`, `DerivedHandler` comments; grep).
- `bind_macro/src/lib.rs`: in the emissions of `descend_impl`, `derived_node_impl`, `derived_enum_node_impl`, and `derived_child_descent`, change the emitted tokens `::bind::Descend` → `::bind::DispatchIntoParent` and the emitted `fn dispatch` / `::dispatch(` calls → `dispatch_into_parent`.
- `bind_macro/src/lib.rs`: rename the generator fn `descend_impl` → `dispatch_into_parent_impl` and update its doc comment. The other internal helpers (`derived_child_descent`, `derived_dispatch_descent`) keep their names: they describe the descent phase of dispatch, not the trait.
- `Dispatch`, the free `dispatch`, and the check half (`EventHandler` / `DerivedHandler`) are untouched.
- Acceptance: `grep -rw Descend crates/` returns nothing; the workspace compiles; bind tests pass unchanged; a grep of mercury for `Descend` confirms no handwritten call sites existed.

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

The handlers, all user-written:

```rust
/// B's bind: go home.
fn go_home<'a, 'c, 'x>(
    _ev: &KeyEvent,
    mut st: AscendState<'a, 'c, BPath<'x>>,
) -> (Vec<DemoEffect>, AscendState<'a, 'c, BPath<'x>>) {
    let state = st.state;
    st.state = match state {
        MaybeInvalidated::NotInvalidated(b) => {
            MaybeInvalidated::Invalidated(b.into_parent().complete()) // Up(a)
        }
        other => other,
    };
    (vec![], st)
}

/// A's bind.
fn flash<'a, 'c, 'x>(
    _ev: &KeyEvent,
    st: AscendState<'a, 'c, APath<'x>>,
) -> (Vec<DemoEffect>, AscendState<'a, 'c, APath<'x>>) {
    (vec![DemoEffect::FlashOverlay], st)
}

/// A's pre: runs before descending into B, while the old timer id is live.
/// A pre takes `&Self::Path` at every depth (field access auto-derefs through
/// the root's `&&mut A` as through `&PathMut`).
fn snap_return_home(_ev: &KeyEvent, a: &APath<'_>) -> TimerId {
    a.b.return_home.id
}

/// A's post: the whole return-home deadline. B on the active path → push the
/// deadline out. Invalidated → the snap is all that is left of the timer.
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
        MaybeInvalidated::Invalidated(_) => vec![DemoEffect::CancelTimer(snapped)],
    };
    (effects, st)
}
```

`Stop` never appears in user code. Staying put is handing the state back; leaving is one field write; branches where an action makes no sense pass the state through.

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
        // Snapped before descent: the schedule is final.
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
                Some((ev, snap_return_home(ev, &path)))
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

`descend_impl` (post-rename: `dispatch_into_parent_impl`), before:

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

Before: each check builds its trigger inline, after the recursion. After: one opt local per scheduled attribute, emitted before the descent, numbered in source order; a `#[pre_post]` opt runs its pre here, so the snap reads pre-descent state:

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

bind gains `AscendState` and `exclusive`; laserbeam gains `MaybeInvalidated` (+ `finish`) and `to_maybe_invalidated` (code above). The check emission, before: the change-1 `*effs = collect(..)` form. After, one kind-blind block per scheduled item; `#rhs` is the attribute's rhs tokens, taken raw for `#[post]`/`#[pre_post]` and wrapped as `::bind::exclusive(#tokens)` for `#[bind]`, and the macro never looks inside:

```rust
if let ::core::option::Option::Some(payload) = opt_N {
    let (e, next) = (#rhs)(payload, st);
    ::core::iter::Extend::extend(effs, e);
    st = next;
}
```

Every bind handler in mercury and the bind tests migrates: `(ev, Node<P, ()>) -> impl IntoIterator<Item = E>` becomes the scheduled shape, `(ev, AscendState<P>) -> (Vec<E>, AscendState<P>)`, handing the state back where the body stays put.

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

All three attribute kinds feed one scheduled list in source order; the differences are confined to parse time (which payload the opt captures, which wrapper the rhs tokens get). `claimed_triggers` does not change: only `#[bind]` triggers claim, so posts are exempt from the duplicate-trigger check.

### Change 5 — derived levels

Derived-level binds migrate to the scheduled shape over `AscendState<Self::Parent>`, and posts across derived-child edges get a story; `DispatchIntoParent`'s `Here` collapse currently hides child-alive from the caller.

## Walks

### KeyH: B goes home

```text
B:  exclusive(go_home): claim won → state := Invalidated(Up(a))
    st.finish() → the stored Completed
A:  st.state = Invalidated(Here(path))
    exclusive(flash): claim already taken → state untouched, flash never runs
    return_home_deadline: Invalidated → [CancelTimer(snapped)]
    st.finish() → the stored Completed
```

### Any other key: B stays

```text
B:  fallthrough → Here(b)
A:  st.state = NotInvalidated(b.into_parent())
    KeyEsc only: exclusive(flash) fires → [FlashOverlay]; state unchanged
    return_home_deadline: NotInvalidated → [CancelTimer(old), ScheduleTimer(fresh)]
    st.finish() → complete(path)
```

Posts run whether or not anything claimed: they are scheduled by their trigger, not by the claim.

## Rules

1. No stubs.
2. Between nodes: `Completed` / `Stop`, `Here` / `Up`. Inside a node: `AscendState`, built once via `to_maybe_invalidated`, threaded through every scheduled item, finished with `st.finish()`. `Stop` never appears in user code.
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent, one per scheduled attribute, in source order. The schedule is final; every scheduled item runs, and its body decides what each state branch means.
5. Every handler is the scheduled shape, raw, and may leave by placing its completed in `Invalidated`; staying put is handing the state back. `exclusive` means not claimed: it is the claim gate and nothing else.
6. The claim lives inside `AscendState`; only binds claim, so posts are exempt from the duplicate-trigger check.
7. Generated code spells laserbeam and bind items fully qualified; handwritten handlers `use laserbeam::{Complete, MaybeInvalidated};` and `use bind::AscendState;`.

## Tests

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- a three-level tree: `Invalidated` forwards through `st.finish()` unchanged
- a gated `exclusive` passes the state through untouched
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts run without a claim, and on both branches of `MaybeInvalidated`
- pre snap reads pre-descent state even when the descent mutates it
- a fired bind that leaves flips the state to `Invalidated` for later items;
  one that stays hands it back unchanged

## Ordered changes

Prefactors first, each independently shippable. The macro deltas per change are in "bind_macro (before / after)".

### 0 — the standalone `DispatchIntoParent` rename (section above; in flight)

### 1 — macro emits `Completed`: signature, linear body, `dispatch_into_parent_impl`/`derived_node_impl`; laserbeam `From<&mut R> for Completed<&mut R>`

### 2 — opts before descent, source order

### 3 — laserbeam `MaybeInvalidated` (+ `finish`), `to_maybe_invalidated`; bind `AscendState`, `exclusive`; one scheduled block per item; handler migration

### 4 — `#[post]` / `#[pre_post]`: registration, parsing, payload capture

### 5 — derived levels: binds to the scheduled shape; derived-edge posts
