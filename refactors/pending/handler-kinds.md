# pre/post handlers

Not done. Adds two handler positions to `bind` beside the existing exclusive one: `pre`, run on the way down — pushing now-effects and returning a value — and `post`, run on the way up with that value and the same `&mut Path` access, guaranteed to run once if its `pre` ran. The post rides on the path as an `on_into_parent` fn, so `into_parent` runs it on both drivers: the miss-unwind's `Continue`, and `ascend`/`complete` on the fire side, which is `into_parent` in a loop and so runs every crossed post as a winner climbs to the root. Effects land in nesting order — `A_pre, B_pre, winner, B_post, A_post`.

## Sequencing

Two changes, the first shippable alone on master.

- Prefactor (no pre/post, `dispatch-batch-and-complete.md`): the plumbing. `Bindings` drops `type Output` for `type Effect`; dispatch threads one `effs: &mut Vec<M::Effect>` and returns `ControlFlow<(), Path>`. Exclusive handlers `ascend` to the root, mutate through `get_mut`, and return `(V, Completed)` — `Completed`'s only constructor is the ascent, so a handler cannot return without having reached the root. `into_parent` gains the `(nested, sink)` seam. With no posts nothing runs through it, so this is behavior-identical to master.
- Feature (this doc): `#[pre_post]` / `#[pre]` / `#[post]`. `pre` becomes `-> (T, Vec<Effect>)` (now-effects on the way down, `T` bound to `opt_i`); the post rides the path and `into_parent`/`ascend` run it, gathering the fire-side posts to place after the winner.

## Syntax

Three attributes:

```rust
#[node(parent = FooParent)] #[binds(M)]
#[pre_post(AnyKey => (arm, stay))]   // the pair: `arm` returns a value, `stay` receives it
struct Foo { #[resolve_into] bar: Bar }

#[pre(AnyKey => track)]    // pre alone: runs on the way down, its return dropped
#[post(AnyKey => passthru)] // post alone: runs on the way up, takes `()`
```

`#[pre]` and `#[post]` are `#[pre_post]` with one half supplied by the macro: `#[pre]`'s post is `drop`, `#[post]`'s pre is the trigger check returning `()`. `#[pre_post]` is the only form that threads a real value from `pre` to `post`. A node may carry several, and the exclusive `#[bind]`, at once.

`trigger => handler` for the singles, `trigger => (pre, post)` for the pair. `pre`/`post` name the timing, not the intent, so a node reads better with the handlers named for what they do (`arm`/`stay`, `track`, `passthru`) and the attribute supplying the timing.

## What the two handlers are

Both handlers are `fn` items the user writes. What rides the path is a closure `on_into_parent` that captured the node's `opt_i`s and calls the posts — a monomorphized `FnOnce`, generic over its own type `F`, no `Box`, no `Rc`, no `dyn`.

- `pre`: `fn(&Event, Node<&mut P, D>) -> (T, Vec<Effect>)`. Runs on the descent if the trigger matches. Node borrowed (`&mut P`), so it can `get_mut` its own node and `ascend` to READ the root, but not consume. It returns `(T, effects)`: `T` is threaded to the post, the effects are pushed as the descent enters the node (the `A_pre`, `B_pre` of the order).
- `post`: `fn(T, Node<&mut P, D>, Nested) -> Vec<Effect>`. Runs on the way up when `pre` ran, taking `pre`'s return `T` and the SAME `&mut Path` access the pre had — `get_mut` its node, `ascend` to read the root. Returns effects. It runs on the live path: on the fire side `complete` runs it BEFORE the winner's `set_layer`, so the node it reaches is still valid.

`pre`'s return survives the descent as an `Option<T>`: `Some(t)` when the trigger matched, `None` when it did not. The match is decided at the descent, up front — the `Option` records it — so `into_parent` runs `post` on the `Some` and skips the `None`, and nothing re-checks a trigger on the way up. `post` sees `T`, never the `Option`; the `Option` is only how the value is carried. `T` is inferred from `pre`'s return, never written (there is no name for it).

That `Option` is the one code path. The alternative — a match arm that calls `post` and one that does not, per pre/post — is `2^n` arms over a node. One `Option` per pre/post is `n` independent `Option`s and one path.

The two standalone forms are this same machine, not special cases:

- `#[pre]` alone: the `post` is `drop`. `pre` runs for its effect on the node, returns `T`, and `into_parent` calls `drop(t)` and yields no effects.
- `#[post]` alone: the `pre` is the trigger check, returning `()`. `Some(())` when it matched, `None` when it did not; `post` takes `()`.

`Nested` is the one outcome exposed — whether the SUBTREE below this node won — as an enum. The node's own exclusive bind is not part of it: the post runs as the sweep passes up through the node, before the node's own bind is tried.

```rust
enum Nested {
    /// A handler won in the subtree below this node (the sweep is being driven up by `complete`).
    Handled,
    /// The subtree below missed (the sweep is the miss-unwind).
    Missed,
}
```

## The sweep

Dispatch is one pass down and one pass up.

Down. Each node builds its child path (`from_fn`) and recurses. A pre/post node runs its `pre`s here: each matching `pre` pushes its now-effects onto the batch (the `A_pre`, `B_pre` of the order) and binds its `T` into `opt_i: Option<T>` (`Some` iff the trigger matched), which the child path's `on_into_parent` closure captures. No exclusive handler fires on the way down.

No handler ever touches a shared accumulator or sees another handler's effects, and this is enforced by the types, not by convention. Every handler, post or exclusive, RETURNS its effects (`-> Vec<Effect>`, `-> (Vec<Effect>, Completed)`); the only holder of a `&mut Vec<Effect>` is framework code — `into_parent`, `complete`, `dispatch`. The accumulator appears in no user-facing signature, so user code cannot push, pop, clear, or read the batch. There is nothing to defend against with an append-only wrapper, because the capability is absent.

The matching restriction is on the path a handler receives. `from_fn` — the only way to build a child path, and thus the only way to choose an `on_into_parent` — is framework-only (crate-private or sealed). A `pre` and a `post` get `Node<&mut P>` (`get_mut`, `ascend`); an exclusive's `Node<P>` gives `ascend` (to `AtRoot`, then `get_mut`/`complete`); none expose `from_fn`. So a handler cannot build a nested path and smuggle in an `on_into_parent` that would pop the batch — the descent, and the choice of every post, stays in generated code.

Up. The recursion unwinds leaf to root, threading one `effs: &mut Vec<Effect>` — the batch. Every level's post is stored on its child's path, so `into_parent(nested, sink)` — ascending out of a child, back into the node — runs the node's post and PUSHES its effects onto `sink`, returning just the parent. Two things drive that ascent:

- the miss-unwind: a subtree that bound nothing returns `Continue`, and the `Continue` arm calls `into_parent(Nested::Missed, effs)`, pushing the level's post effects straight onto the batch.
- `ascend`/`complete`: when a bind matches, its handler climbs to the root through `ascend`, which loops `into_parent(Nested::Handled, ..)` — running every crossed post ON THE LIVE PATH, before the winner's `set_layer`. It gathers those posts into a scratch vec the handle OWNS, and `complete` moves that into the `Completed` token. The handler mutates the root through `get_mut`, returns `(its own effects, Completed)`, and the framework pushes the winner's own effects onto `effs` FIRST, then drains the token's gathered posts. The whole thing propagates up as `Break(())` without any further `into_parent` — `ascend` already climbed those levels.

Effects land in nesting order — `A_pre, B_pre, winner, B_post, A_post`: the pres pushed on the way down, the winner next, then the posts `ascend` gathered on the live path but placed after the winner. Safe because the gathered posts are computed VALUES by the time `set_layer` runs; they never re-touch the swapped layer.

```rust
// Dispatch for a node, sketch. One `effs: &mut Vec<Effect>` is threaded; ControlFlow<(), Path>.
let child_path = from_fn(node_path, /* projections + on_into_parent closure capturing this node's opt_i */);
match Child::Dispatch::dispatch(child_path, event, effs) {
    // subtree won: ascend already ran this node's post (Handled) as it climbed; just propagate
    Break(()) => Break(()),
    Continue(child_path) => {
        // subtree missed: ascend into this node, running its post as Missed (pushed onto effs)
        let node_path = child_path.into_parent(Nested::Missed, effs);
        // now try this node's own exclusive binds against node_path (see below). On a match:
        //   let (own, done) = handler(..);       // handler returns its own effects + the token
        //   effs.extend(own);                     // winner first
        //   effs.extend(done.into_gathered());    // then the posts it climbed past
        //   return Break(());
        Continue(node_path)
    }
}
```

## `ascend`, `AtRoot`, `Completed`

The feature adds the gathered posts to the prefactor's `ascend`/`AtRoot`/`Completed`. `ascend::<Root>()` still climbs to the root and hands back `AtRoot` — but the climb now loops `into_parent(Nested::Handled, ..)`, so it runs every crossed post into a scratch vec `AtRoot` owns. `get_mut` mutates the root; `complete` moves the gathered posts into the proof token:

```rust
pub struct AtRoot<'a> {
    root: &'a mut Root,
    gathered: Vec<Effect>,   // the posts this climb crossed, in order — the feature's addition
}
impl<'a> AtRoot<'a> {
    pub fn get_mut(&mut self) -> &mut Root { self.root }
    pub fn complete(self) -> Completed { Completed { gathered: self.gathered } }
}

/// Sealed proof, now carrying the gathered posts for the framework to drain. Exposes nothing else.
pub struct Completed { gathered: Vec<Effect> }
impl Completed {
    pub fn into_gathered(self) -> Vec<Effect> { self.gathered }
}
```

`ascend`'s `PathMut` impl owns the scratch vec, `climb`s it (`into_parent(Nested::Handled, &mut gathered)` to the root), and hands back `AtRoot { root, gathered }`. The handler never sees the vec: it mutates through `get_mut` and completes.

```rust
fn to_nav(_ev: &E, node: Node<P>) -> (Vec<MercuryEffect>, Completed) {
    let mut a = node.parent.ascend::<Mercury>();   // climbs, running the crossed posts on the live path
    let fx = a.get_mut().set_layer(nav);           // mutate the root — posts already ran, before this
    (fx, a.complete())                             // own effects, plus the proof (carrying the posts)
}
```

`ascend` consuming the path is what forces the climb: the handler cannot reach `Completed` without having gone all the way up, and every `into_parent` on the way ran that level's post ON THE LIVE PATH — before `set_layer`. Forgetting to ascend, or ascending halfway, does not compile. In the prefactor there are no posts, so `gathered` is always empty and `AtRoot`/`Completed` are the token-carrying `ascend_mut` of `dispatch-batch-and-complete.md`.

## `PathMut`

`PathMut` carries the `on_into_parent` closure. Before:

```rust
pub struct PathMut<'a, N, P> { /* projection to N, parent P */ }

impl<'a, N, P> HasParent for PathMut<'a, N, P> {
    type Parent = P;
    fn into_parent(self) -> P { /* project up */ }
}
```

after (`F` is the `on_into_parent` closure, inferred; it CLOSES OVER the pre values — there is no separate `held` field and no `T` parameter):

```rust
pub struct PathMut<'a, N, P, F> {
    /* projection to N, parent P (fn pointers, no capture) */
    on_into_parent: F,   // F: FnOnce(Node<&mut P, N>, Nested) -> Vec<Effect>; captures the pre values
}

impl<'a, N, P, F> PathMut<'a, N, P, F>
where
    F: FnOnce(Node<&mut P, N>, Nested) -> Vec<Effect>,
{
    /// Ascend one level, running its `on_into_parent` once and PUSHING what it returns onto `sink`.
    /// `into_parent` consumes the level, so the `FnOnce` runs at most once. Returns just the parent.
    pub fn into_parent(self, nested: Nested, sink: &mut Vec<Effect>) -> P {
        // `self.node()` reborrows the level as a `Node` — the pre's `&mut Path` access
        Extend::extend(sink, (self.on_into_parent)(self.node(), nested));
        self.parent
    }
}

/// The default a node with no pre/post gets: an `on_into_parent` that captures nothing and yields
/// nothing.
fn no_post<N, P>(_: Node<&mut P, N>, _: Nested) -> Vec<Effect> { Vec::new() }
```

The closure runs INSIDE `into_parent`, so it runs on both drivers through the one funnel: the miss-unwind's `Continue` (`Nested::Missed`) and `complete`'s climb on the fire side (`Nested::Handled`). That is the whole reason `complete` exists — it is `into_parent` in a loop, so a winner firing deep runs every level's `on_into_parent` above it, on the live path, before `set_layer`.

`from_fn` takes the `on_into_parent` closure — the child path is built with it, never mutated afterward. A pre/post node passes the closure that captured its `opt_i`; every other descent passes `no_post`. A node with several pre/posts captures every `opt_i` in the ONE closure, which runs each post — one closure per node, no stack of levels, no tuple.

## The generated `Dispatch`

For `#[pre_post(AnyKey => (arm, stay))] Foo { #[resolve_into] bar: Bar }` — one pre/post. The child path's `on_into_parent` is a closure that captured `opt_0` and calls `stay`:

```rust
impl Dispatch<M> for Foo {
    fn dispatch<'a>(mut path: <Foo as Place>::Path<'a>, event: &M::Event, effs: &mut Vec<Effect>)
        -> ControlFlow<(), <Foo as Place>::Path<'a>>
    {
        // down: run the pre if its trigger matches. It returns `(T, now-effects)`: push the
        // now-effects, bind `T` into `opt_0` (`None` if the trigger missed — decided HERE).
        let opt_0 = match <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if AnyKey.is_matching(ev) => {
                let (t, now) = arm(ev, ::bind::Node { parent: &mut path, data: () });
                ::core::iter::Extend::extend(effs, now);
                Some(t)
            }
            _ => None,
        };

        // descend. The post rides the child path as `on_into_parent`, a CLOSURE that captures
        // `opt_0` and calls `stay`. A second `#[pre_post]` binds `opt_1` above and the SAME closure
        // runs it too — one closure per node, not a stack of levels.
        let bar_path = ::laserbeam::PathMut::from_fn(
            path, |p| &mut p.get_mut().bar, |p| &p.get().bar,
            move |node, nested| match opt_0 {
                ::core::option::Option::Some(t) => stay(t, node, nested),
                ::core::option::Option::None => ::std::vec::Vec::new(),
            },
        ).into();

        // up. (Foo has no exclusive bind of its own; a node that did would try it in the Continue
        // arm, below.)
        match <Bar as ::bind::Dispatch<M>>::dispatch(bar_path, event, effs) {
            ::core::ops::ControlFlow::Break(()) => ::core::ops::ControlFlow::Break(()),
            ::core::ops::ControlFlow::Continue(bar_path) =>
                ::core::ops::ControlFlow::Continue(bar_path.into_parent(::bind::Nested::Missed, effs)),
        }
    }
}
```

A node WITH an exclusive bind tries it in the `Continue` arm, against the recovered path. The winner fires via `ascend`/`complete`, which climbs to the root running the crossed posts, and the level returns `Break` so ancestors propagate without ascending again:

```rust
// inside the Continue arm, after the subtree missed and `path` was recovered. `effs` already holds
// the pres from the way down, then the below-winner posts.
if let Some(ev) = <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
    if Key::KeyN.down().is_matching(ev) {
        // to_nav returns its own effects plus the token; the token carries the posts it climbed past
        let (own, done) = to_nav(ev, ::bind::Node { parent: path, data });
        ::core::iter::Extend::extend(effs, own);                    // winner first
        ::core::iter::Extend::extend(effs, done.into_gathered());   // then the posts it climbed (Handled)
        return ::core::ops::ControlFlow::Break(());
    }
}
::core::ops::ControlFlow::Continue(path)
```

## The guarantee

`pre` ran ⟹ `post` ran, exactly once. The `on_into_parent` closure rides on the child path from construction, and there is exactly one ascent through that level — one `into_parent`, in the miss-unwind's `Continue` arm or inside `complete`. `into_parent` consumes the level and calls the `FnOnce` once; the captured `opt_i` moves into it and cannot be run twice. The once-ness is the ownership, not a flag.

## The rearm as a user

`AndReturnHome { layers, guard }`, `#[pre_post(AnyKey => (arm, stay))]` plus the exclusive firing. `arm` returns `(schedule, no now-effects)`; `stay` receives the schedule as `T` on the way up and emits it:

```rust
// pre: mint a fresh timer, replace the old guard (cancelling the old timer). Return the schedule as
// `T` (threaded to `stay`) and no now-effects — the reschedule goes out on the way UP, after the
// winner. The guard stays on the node for its `Drop`.
fn arm(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath, ()>) -> (MercuryEffect, Vec<MercuryEffect>) {
    let (guard, schedule) = arm_return_home();
    node.parent.get_mut().guard = guard;
    (schedule, vec![])
}

// post: emit the schedule `arm` handed over. Gets the same `Node<&mut Path>` as `arm`; unused here.
fn stay(schedule: MercuryEffect, _node: Node<&mut AndReturnHomePath, ()>, _nested: Nested) -> Vec<MercuryEffect> {
    vec![schedule]
}
```

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over the existing tree:

- `pre` then `post` on a miss: an event the subtree does not bind runs `pre` (pushes its now-effects, binds `opt_0`), descends, misses, and the `Continue` arm's `into_parent` runs `post` with `Nested::Missed`; its effect lands on the threaded `effs`.
- `post` on a handled subtree: a leaf exclusive wins and `ascend` climbs; `post` runs with `Nested::Handled` as the climb crosses the pre/post node, and its effect lands AFTER the winner's own — the batch reads `A_pre, B_pre, winner, B_post, A_post`.
- exactly-once: a drop counter on `pre`'s returned value shows it is consumed once, never twice and never zero, across both drivers.
- `pre` did not match: an event whose trigger `pre` rejects binds `None`, and `into_parent` runs no `post`.
- the exclusive winner is unchanged: leafward-most still wins, and cannot return without a `Completed`.
- the post runs on the LIVE path: a `post` that `ascend`s to read the root, on the fire side, sees the pre-`set_layer` root (the climb runs before the winner mutates).

## Status

The work, prefactor first:

- prefactor, on master (`dispatch-batch-and-complete.md`): `Bindings` drops `type Output` for `type Effect`; dispatch threads `effs: &mut Vec<M::Effect>` and returns `ControlFlow<(), Path>`. laserbeam adds `ascend`/`AtRoot`/`Completed` and the `Nested`/`sink` seam on `into_parent`; exclusive handlers `ascend`, mutate through `get_mut`, and return `(V: Into<Vec<Effect>>, Completed)`. `from_fn` becomes framework-only. Nothing runs through the seam yet, so behavior-identical to master.
- feature (this doc): `PathMut` gains `on_into_parent: F` (a `FnOnce(Node<&mut P, N>, Nested) -> Vec<Effect>` closure that captured the node's `opt_i`s) — no `held` field, no `T` param. `into_parent` runs it through the seam; `ascend`/`complete` gather the crossed posts. `pre` becomes `-> (T, Vec<Effect>)`. `bind_macro` gains `#[pre_post]` / `#[pre]` / `#[post]` — the descent-time pre binding `opt_i` and pushing now-effects, `from_fn` storing the closure.

The `&mut Vec<Effect>` batch appears in no user-facing signature — posts and handlers return effects, the framework pushes — so user code cannot pop or read the batch, and no append-only wrapper is needed.

Open surface question: naming (`pre_post` vs two attributes; timing vs intent names).
