# Invalidation: descent schedules, ascent runs posts

Not done. Prefactor `path-peel-complete.md`: landed. Invalidation change "Claim + effs sink, drop ControlFlow" (22e5580): landed. Every "before" below is the code on disk after that commit.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post.

## Model

Every node's dispatch returns `Completed<Self::Path<'a>>`, root included. There is no per-node ascent type and no associated type: the return is the same expression of the node's own path everywhere.

A non-root parent sees three outcomes, from two nested `into_inner` matches:

```rust
match Child::dispatch(child_path, event, effs, claim).into_inner() {
    Stop::Here(child_path) => {
        // child kept focus; this node's path is child_path.into_parent()
    }
    Stop::Up(rest) => match rest.into_inner() {
        Stop::Here(path) => {
            // the leave stopped at this node; child dropped, this node lives
        }
        Stop::Up(above) => {
            // this node dropped too; forward Completed::up(above)
        }
    },
}
```

The `Up` payload of a child's `Completed` is this node's own `Completed`, so the no-inspection form forwards `rest` unchanged; the inspecting form rebuilds its gone-above arm with `Completed::up`. At the root the child's `Up` payload is the bare root path, so the root sees two arms.

"Dropped" means dropped from the active path: focus left it. Whether its state was also replaced is the handler's business (an enum layer usually swaps; a struct field persists); posts key on the active path either way.

Posts per arm:

```text
Here(child)            child active      → alive posts (live path); own binds (claim-gated)
Up(Here(this))         child dropped     → dropped posts (snap bodies; this node's path is live)
Up(Up(above))          this node dropped → dropped posts from pre-descent snaps only
```

A bind is not special: `#[bind(X => foo)]` desugars to `#[post(X => exclusive(foo))]`, where `exclusive` is a handler combinator that calls `foo` iff it can establish the claim and otherwise hands the path back. The macro is dumb: scheduled items get one emission shape, and the attribute kind only decides which adapter wraps the rhs tokens (`exclusive(rhs)` for `#[bind]`, `stays(rhs)` for `#[post]`). Binds appear only in the kept arm: an `Up` is only ever produced by a handler, every handler runs inside `exclusive`, so in any `Up` arm the claim is already taken and the wrapper could never fire.

## Landed baseline (no further change)

`bind/src/lib.rs` already holds `Claim` (`try_take` stays if/else because it is `const fn` and `Option::replace` is not const; `exclusive` is added in change 3), the final `Dispatch` and `Descend` signatures, and the final free `dispatch`:

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

`A` is the root and holds the layer `B`. `B` arms a return-home timer; every key while `B` is up pushes the deadline out; leaving `B` must cancel the timer, because the OS timer outlives the state that armed it and `Drop` cannot emit the cancel.

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
// #[post] (alive arms, live path) and #[pre_post] (dropped arms, snap only)
// are new in change 4. Opts are numbered in source order across all three
// kinds: a bind is a post that claims, not a special case.
#[derive(Bind)]
#[node(root)]
#[binds(M)]
#[bind(KeyEsc => flash)]                                               // opt_0
#[post(AnyKey => rearm)]                                               // opt_1
#[pre_post(AnyKey => (snap_return_home, cancel_return_home))]          // opt_2
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

The handlers and posts, all user-written:

```rust
/// B's bind: go home. The layer is replaced, so B and the guard it holds drop.
fn go_home<'a>(
    _ev: &KeyEvent,
    node: Node<BPath<'a>, ()>,
) -> (Vec<DemoEffect>, Completed<BPath<'a>>) {
    (vec![], node.parent.into_parent().complete()) // Up(a)
}

/// A's bind: fires only when nothing deeper claimed the key.
fn flash<'a>(
    _ev: &KeyEvent,
    node: Node<APath<'a>, ()>,
) -> (Vec<DemoEffect>, Completed<APath<'a>>) {
    (vec![DemoEffect::FlashOverlay], node.parent.complete())
}

/// A's post while B is alive: any key pushes B's return-home deadline out.
/// `&mut Self::Path` at every depth, root included.
fn rearm(a: &mut APath<'_>) -> Vec<DemoEffect> {
    let fresh = TimerId::fresh();
    let old = core::mem::replace(&mut a.b.return_home, TimerGuard { id: fresh });
    vec![DemoEffect::CancelTimer(old.id), DemoEffect::ScheduleTimer(fresh)]
}

/// A's pre: runs before descending into B, while B still exists.
fn snap_return_home(_ev: &KeyEvent, a: &A) -> TimerId {
    a.b.return_home.id
}

/// A's post when B dropped: the snap is all that is left of the timer.
fn cancel_return_home(id: TimerId) -> Vec<DemoEffect> {
    vec![DemoEffect::CancelTimer(id)]
}
```

The user never writes an arm match and never sees `Stop`: which body runs on which arm is declared (`post` = alive, `pre_post` = dropped) and the macro emits the arms.

## Generated: B (target, after change 4)

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

        if let Some(ev) = opt_0 {
            let (e, flow) = (::bind::exclusive(go_home))(ev, ::bind::Node { parent: path, data: () }, claim);
            effs.extend(e);
            path = match flow {
                ControlFlow::Break(completed) => return completed,
                ControlFlow::Continue(p) => p,
            };
        }

        ::laserbeam::Complete::complete(path) // Here(b)
    }
}
```

## Generated: A (target, after change 4)

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
        // Pre-descent, in source order: snaps and the schedule, final before B runs.
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

        let opt_1: Option<&KeyEvent> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(ev)
            } else {
                None
            }
        } else {
            None
        };

        let opt_2: Option<TimerId> = if let Ok(ev) = TryFrom::try_from(event) {
            let trigger = AnyKey;
            if EventTrigger::is_matching(&trigger, ev) {
                Some(snap_return_home(ev, path))
            } else {
                None
            }
        } else {
            None
        };

        let b_path = laserbeam::PathMut::from_fn(path, |a| &mut a.b, |a| &a.b);

        match B::dispatch(b_path, event, effs, claim).into_inner() {
            Stop::Here(b_path) => {
                let mut path = b_path.into_parent();
                if let Some(ev) = opt_0 {
                    let (e, flow) = (::bind::exclusive(flash))(ev, ::bind::Node { parent: path, data: () }, claim);
                    effs.extend(e);
                    path = match flow {
                        ControlFlow::Break(completed) => return completed,
                        ControlFlow::Continue(p) => p,
                    };
                }
                if let Some(ev) = opt_1 {
                    let (e, flow) = (::bind::stays(rearm))(ev, ::bind::Node { parent: path, data: () }, claim);
                    effs.extend(e);
                    path = match flow {
                        ControlFlow::Break(completed) => return completed,
                        ControlFlow::Continue(p) => p,
                    };
                }
                ::laserbeam::Complete::complete(path)
            }
            Stop::Up(path) => {
                // B dropped; the leave stopped here at the root.
                if let Some(id) = opt_2 {
                    effs.extend(cancel_return_home(id));
                }
                ::laserbeam::Complete::complete(path)
            }
        }
    }
}
```

A deeper tree gets the three-arm match from Model: alive posts in both live arms, snap-only posts in the gone-above arm, `Completed::up(above)` forwarding.

## bind_macro (before / after)

### Change 1 — emit `Completed`; forwarding matches

laserbeam gains one impl (compile-checked in the design scratch alongside `Completed::up`), so generated code can normalize an `Up` payload that is either the bare root path or already a `Completed` with one `Into::into`:

```rust
/// The root's completed leave as a conversion, so generated code normalizes an
/// `Up` payload (the bare root path, or already a `Completed`) with one
/// `Into::into`, whichever the parent slot holds.
impl<'a, R> From<&'a mut R> for Completed<&'a mut R> {
    fn from(root: &'a mut R) -> Self {
        Completed::new(root)
    }
}
```

`dispatch_impl`, emitted signature and fallthrough, before:

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
    #recurse
    #(#checks)*
    ::laserbeam::Complete::complete(path)
}
```

The bind checks, before (claim landed; handler still returns bare effects; a fired bind ends dispatch with `None`):

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

After (a fired bind stays put until change 3 lets the handler say otherwise):

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
            return ::laserbeam::Complete::complete(path);
        }
    }
}
```

`dispatch_body`'s struct-child recurse, before (the `?` rides the `Option`):

```rust
let child = <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim)?;
path = #recover;
```

After — forwarding only; the arm split for posts arrives in change 4. `#recover` is `Edge::recover_parent`, unchanged:

```rust
path = match ::laserbeam::Completed::into_inner(
    <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim),
) {
    ::laserbeam::Stop::Here(child) => #recover,
    ::laserbeam::Stop::Up(rest) => return ::core::convert::Into::into(rest),
};
```

`Into::into` covers both parent shapes: `rest` is `Completed<Self::Path>` at a non-root node (reflexive `From`) and the bare root path at a child of the root (the new laserbeam impl). The enum-child case applies the same transform inside each variant arm. `descend_impl`, before:

```rust
match <#name as ::bind::Dispatch<#marker>>::dispatch(self, event, effs, claim) {
    ::core::option::Option::None => ::core::option::Option::None,
    ::core::option::Option::Some(path) => {
        ::core::option::Option::Some(::bind::HasParent::into_parent(path))
    }
}
```

After:

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

Collapsing `Here(child)` into `Here(parent)` here matches today's `Continue` behavior; posts across derived edges are change 5's concern. `derived_node_impl`'s fallthrough transforms the same way: `Continue(HasParent::into_parent(node))` becomes `Complete::complete(HasParent::into_parent(node))`, hmm its parent is a path, so `::laserbeam::Complete::complete(::bind::HasParent::into_parent(node))`.

### Change 2 — opts before descent, source order

Before: each check builds its trigger inline, after the recursion (the snippet above). After: one `opt_N` local per scheduling attribute, emitted before `#recurse`, numbered in source order across `#[bind]`, `#[post]`, `#[pre_post]` alike; the checks consume the opts:

```rust
let opt_N: ::core::option::Option<_> =
    match ::core::convert::TryFrom::try_from(event) {
        ::core::result::Result::Ok(ev) => {
            let trigger = #trigger;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(ev)
            } else {
                ::core::option::Option::None
            }
        }
        ::core::result::Result::Err(_) => ::core::option::Option::None,
    };
```

A `#[pre_post]` opt's `Some` arm is `Some(#pre(ev, &path))` — the snap runs here, before the descent, which is the semantic point of this change: the schedule and its snaps are final before the child can mutate anything.

### Change 3 — one scheduled-post shape; `exclusive` / `stays` adapters

bind gains two free functions; they are the whole difference between a bind and a post. Both produce the one scheduled-post shape the macro calls:

```rust
/// Wraps a bind handler into the scheduled-post shape: the handler runs iff
/// the claim can be established; otherwise the path is handed back untouched.
/// `#[bind(X => foo)]` is `#[post(X => exclusive(foo))]`.
pub fn exclusive<Ev, P, D, E>(
    handler: impl FnOnce(&Ev, Node<P, D>) -> (Vec<E>, ::laserbeam::Completed<P>),
) -> impl FnOnce(&Ev, Node<P, D>, &mut Claim<'_>) -> (Vec<E>, ControlFlow<::laserbeam::Completed<P>, P>)
where
    P: ::laserbeam::HasStop,
{
    move |ev, node, claim| match claim.try_take() {
        Some(()) => {
            let (e, completed) = handler(ev, node);
            (e, ControlFlow::Break(completed))
        }
        None => (Vec::new(), ControlFlow::Continue(node.parent)),
    }
}

/// Wraps a plain effects post into the same shape: always runs, never claims,
/// never moves focus. `#[post(X => f)]` schedules `stays(f)`.
pub fn stays<Ev, P, D, E>(
    post: impl FnOnce(&mut P) -> Vec<E>,
) -> impl FnOnce(&Ev, Node<P, D>, &mut Claim<'_>) -> (Vec<E>, ControlFlow<::laserbeam::Completed<P>, P>)
where
    P: ::laserbeam::HasStop,
{
    move |_ev, mut node, _claim| {
        let e = post(&mut node.parent);
        (e, ControlFlow::Continue(node.parent))
    }
}
```

(`ControlFlow` is the combinator's return, not the dispatch surface: `Break` ends this node's dispatch with the handler's `Completed`, `Continue` is the path handed back.)

The macro wraps the rhs tokens by attribute kind at parse time and emits one kind-blind block per scheduled item. Before: the change-1 check form above (`*effs = collect(..); return Complete::complete(path)`). After:

```rust
if let ::core::option::Option::Some(ev) = opt_N {
    let (e, flow) = (#rhs)(ev, ::bind::Node { parent: path, data: () }, claim);
    ::core::iter::Extend::extend(effs, e);
    path = match flow {
        ::core::ops::ControlFlow::Break(completed) => return completed,
        ::core::ops::ControlFlow::Continue(p) => p,
    };
}
```

`#rhs` is `::bind::exclusive(#tokens)` for `#[bind]` and `::bind::stays(#tokens)` for `#[post]`; nothing downstream of parsing knows which kind it is.

Every bind handler in mercury and the bind tests changes signature mechanically: `-> impl IntoIterator<Item = E>` becomes `-> (Vec<E>, Completed<Path>)`, with `path.complete()` appended to every body that stays put. Posts are `fn(&mut Self::Path) -> Vec<E>` at every depth, root included.

### Change 4 — `#[post]` / `#[pre_post]`

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

Parsing, beside `Binding` (whose `Parse` the `Post` form reuses):

```rust
/// One `trigger => handler` pair from `#[post(..)]`: runs on the alive arms
/// with the live path.
struct Post {
    trigger: Expr,
    handler: Expr,
}

/// One `trigger => (pre, post)` pair from `#[pre_post(..)]`: pre snaps before
/// descent, post runs on the dropped arms with the snap.
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

fn posts(attrs: &[syn::Attribute]) -> syn::Result<Vec<Post>>;        // mirrors binds()
fn pre_posts(attrs: &[syn::Attribute]) -> syn::Result<Vec<PrePost>>; // mirrors binds()
```

The struct-child recurse grows from change 1's forwarding into the full arms. Before: the change-1 `path = match … return Into::into(rest)` form. After, non-root:

```rust
match ::laserbeam::Completed::into_inner(
    <#child as ::bind::Dispatch<#marker>>::dispatch(#child_path, event, effs, claim),
) {
    ::laserbeam::Stop::Here(child) => {
        let mut path = #recover;
        #(#alive_posts)*
        #(#checks)*
        ::laserbeam::Complete::complete(path)
    }
    ::laserbeam::Stop::Up(rest) => match ::laserbeam::Completed::into_inner(rest) {
        ::laserbeam::Stop::Here(mut path) => {
            #(#dropped_posts)*
            ::laserbeam::Complete::complete(path)
        }
        ::laserbeam::Stop::Up(above) => {
            #(#dropped_posts)*
            ::laserbeam::Completed::up(above)
        }
    },
}
```

Root: one `Up` arm, `dropped_posts` then `Complete::complete(path)` (the payload is the bare root path). Each `alive_post` is `if opt_N { Extend::extend(effs, #handler(path or &mut path)); }` (bare `path` at the root, `&mut path` on a `PathMut`); each `dropped_post` is `if let Some(snap) = opt_N { Extend::extend(effs, #post(snap)); }`.

`claimed_triggers` does not change: posts and pre_posts never claim, so they are exempt from the duplicate-trigger check.

### Change 5 — remaining refinements

Here-only path-mutation posts; posts across derived-child edges (where `Descend`'s `Here` collapse currently hides child-alive from the caller).

## Walks

### KeyH: B goes home

```text
B:  claim take; go_home returns node.parent.into_parent().complete() → Up(a)
A:  Up(path); cancel_return_home(snapped id) → [CancelTimer]
    return path.complete()
```

### Any other key: B stays

```text
B:  fallthrough → Here(b)
A:  Here(b) → path = b.into_parent(); rearm(path)
    → [CancelTimer(old), ScheduleTimer(fresh)]
    KeyEsc additionally: flash claims → [FlashOverlay], returns Here
```

`rearm` runs whether or not anything claimed: posts are scheduled by their trigger, not by the claim.

## Rules

1. No stubs.
2. Arms `Here` / `Up`; three outcomes at a non-root parent; forwarding without inspection is `return Into::into(rest)`.
3. Every dispatch returns `Completed<Self::Path>` (derived levels: `Completed<Self::Parent>`); no ascent associated type.
4. Opts are snapped before descent, one per scheduling attribute, numbered in source order; a bind is a post that claims, not a special case. The schedule is final; ascent runs every scheduled post.
5. `#[bind(X => foo)]` desugars to `#[post(X => exclusive(foo))]`; `#[post(X => f)]` schedules `stays(f)`; one emission shape, the attribute kind decides only the wrapper. Bind handlers return `(effects, Completed<Self::Path>)` and staying put is `path.complete()`; posts are `fn(&mut Self::Path) -> Vec<E>`.
6. Posts and pre_posts never claim and are exempt from the duplicate-trigger check.
7. The user writes triggers, handlers, pres, and posts; the macro writes every arm match. `Stop` never appears in user code.
8. Generated code spells laserbeam items fully qualified; handwritten handlers `use laserbeam::Complete;`.

## Tests

- KeyH / any-key walks on the A/B expansion, asserting the exact effect
  sequences above
- three-arm coverage on a three-level tree (kept / stopped-at-mid / gone-above)
- claim trap door: KeyEsc bound at A fires only when B did not claim
- posts fire without a claim (rearm on an unbound key)
- pre_post snap reads pre-descent state even when the descent mutates it

## Ordered changes

Prefactors first, each independently shippable. The macro deltas per change are in "bind_macro (before / after)".

### 1 — macro emits `Completed`: signature, fallthrough `complete()`, forwarding matches, `descend_impl`/`derived_node_impl`; laserbeam `From<&mut R> for Completed<&mut R>`

### 2 — opts before descent, source order, pre_post snaps run here

### 3 — bind handlers return `(effects, Completed<Path>)`; mercury/tests signatures migrate

### 4 — `#[post]` / `#[pre_post]`: registration, parsing, full arm emission, check exemption

### 5 — Here-only path posts; derived-edge posts
