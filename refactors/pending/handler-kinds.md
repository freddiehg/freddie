# pre/post handlers

Not done. Adds two handler positions to `bind` beside the existing exclusive one: `pre`, run on the way down and returning a value, and `post`, run on the way up with that value, guaranteed to run once if its `pre` ran. Dispatch becomes one descent and one leaf-to-root ascent: the pres arm on the way down, the posts run on the way up, and every source returns its effects, concatenated into one `Vec<Effect>`.

## Sequencing

Two changes, the first shippable alone on master.

- Prefactor (no pre/post): the return shape. `Bindings` drops `type Output` for `type Effect`; dispatch returns `Vec<M::Effect>`. Exclusive handlers return `(Vec<Effect>, Completed)` instead of bare effects, and reach the root through `complete` instead of `ascend_mut` — `complete` is `ascend_mut` plus a token whose only constructor is the ascent, so a handler cannot return without having reached the root. With no posts nothing on the ascent emits, so `complete` returns no effects and this is behavior-identical to master.
- Feature (this doc): `#[pre_post]` / `#[pre]` / `#[post]`. Pres arm `opt_i` on the descent; `into_parent` and `complete` now run the crossed posts, so their returned `Vec`s carry effects.

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

Both handlers are `fn` items, not closures. That is the whole reason this is cheap: what gets stored is a `fn` pointer, `Copy`, no `Box`, no `Rc`, no `dyn`.

- `pre`: `fn(&Event, Node<&mut P, D>) -> T`. Runs on the descent if the trigger matches. Node borrowed (`&mut P`), so it can `get_mut` its own node and `ascend` to READ the root, but not consume. It RETURNS `T`.
- `post`: `fn(T, &mut N, Nested) -> Vec<Effect>`. Runs on the way up when `pre` ran, taking `pre`'s return `T`, and returns effects — the same shape as an exclusive handler. It does not touch any shared accumulator; the caller concatenates what it returns.

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

Down. Each node builds its child path (`from_fn`) and recurses. A pre/post node runs its `pre`s here, binding each to `opt_i: Option<T>` (`Some` iff the trigger matched) and storing them on the child path beside the posts. Nothing fires and nothing emits on the way down.

No handler ever touches a shared accumulator or sees another handler's effects. Every handler, post or exclusive, returns only its OWN effects; the framework does every concatenation. That is the whole ergonomic goal, and it is why `complete` gathers rather than the handler.

Up. The recursion unwinds leaf to root. Every level's post is stored on its child's path, so `into_parent` — ascending out of a child, back into the node — runs the node's post and hands back `(Vec<Effect>, ParentPath)`. Two things drive that ascent:

- the miss-unwind: a subtree that bound nothing returns `Continue`, and the `Continue` arm `into_parent`s with `Nested::Missed`, concatenating the level's post effects below what it already has.
- `complete`: when a bind matches, its handler ascends to the root through `complete`, which is `into_parent` in a loop with `Nested::Handled`. It GATHERS the crossed posts into the `Completed` token it returns — the handler never sees them. The handler returns `(its own effects, Completed)`, and the framework drains the token's gathered posts and then the handler's own effects into the batch. The whole thing propagates up as `Break` without any further `into_parent` — `complete` already climbed those levels.

Effects land in crossing order, posts before the winner: the misses below accumulate up to the winning level, the token's gathered posts go next, and the winner's own effects last.

```rust
// Dispatch for a node, sketch. ControlFlow<Break=(Vec, Completed), Continue=(Vec, Path)>.
let child_path = from_fn(node_path, /* this node's post + opt_i from its pre */);
match Child::Dispatch::dispatch(child_path) {
    // subtree won: complete already ran this node's post (Handled) as it climbed; just propagate
    Break(won) => Break(won),
    Continue((below, child_path)) => {
        // subtree missed: ascend into this node, running its post as Missed
        let (mine, node_path) = child_path.into_parent(Nested::Missed);
        let mut effs = below; effs.extend(mine);
        // now try this node's own exclusive binds against node_path (see below). On a match:
        //   let (own, mut done) = handler(..);         // handler returns only its own effects
        //   effs.extend(done.take_gathered());          // framework drains the token's posts
        //   effs.extend(own);                           // then the winner's own
        //   return Break((effs, done));
        Continue((effs, node_path))
    }
}
```

## `complete` and `Completed`

The exclusive winner reaches the root to mutate it. It takes an owned `Node<P>` (never `&mut P` — that would nerf it out of ascending and hand it write access to the parent it is about to walk). It must return a `Completed`, and the only constructor of `Completed` is the ascent. The token also GATHERS the crossed posts, so the handler never handles them:

```rust
// sealed: private field, so `complete` is the ONLY way to make one — a handler cannot fabricate it
pub struct Completed<'a> {
    root: &'a mut Root,     // DerefMut to Root, so the handler mutates through it
    gathered: Vec<Effect>,  // the posts this ascent crossed, in order; the framework drains it
}

impl<'a> Completed<'a> {
    pub fn take_gathered(&mut self) -> Vec<Effect> { core::mem::take(&mut self.gathered) }
}

// in the AscendMut family; every path level and the root path implement it
impl<'a> Complete<'a> for &'a mut Root {                       // base: already at root, nothing gathered
    fn complete(self) -> Completed<'a> { Completed { root: self, gathered: Vec::new() } }
}
impl<'a, ..> Complete<'a> for PathMut<..> {                    // run this level's post, recurse up
    fn complete(self) -> Completed<'a> {
        let (mut gathered, parent) = self.into_parent(Nested::Handled);   // this level's post, lower
        let mut done = parent.complete();                                 // the levels above
        gathered.extend(done.gathered);                                   // crossing order: lower then higher
        done.gathered = gathered;
        done
    }
}
```

So the winner author writes nothing about the posts it climbs past:

```rust
fn to_nav(_ev: &E, node: Node<P>) -> (Vec<MercuryEffect>, Completed) {
    let mut done = node.parent.complete();   // ascends, gathers the crossed posts INTO `done`
    (done.set_layer(nav), done)              // returns only ITS OWN effects, plus the token
}
```

Because `complete` consumes the path, the moment a handler wants its return value it has already walked itself to the root and every `into_parent` on the way ran that level's post. A handler physically cannot return without having ascended and run its half of the sweep — strictly better than plain `ascend_mut`, where forgetting to ascend, or ascending halfway, compiles.

`Completed` gathers effects only because of the posts it crosses. In the prefactor there are none, so `gathered` is always empty and `Completed` degenerates to a token-carrying `ascend_mut`.

## `PathMut`

`PathMut` carries the `post` and its held `Option<T>`. Before:

```rust
pub struct PathMut<'a, N, P> { /* projection to N, parent P */ }

impl<'a, N, P> HasParent for PathMut<'a, N, P> {
    type Parent = P;
    fn into_parent(self) -> P { /* project up */ }
}
```

after (`T` is `pre`'s return, inferred; effects are returned, not written through a `&mut`):

```rust
pub struct PathMut<'a, N, P, T> {
    /* projection to N, parent P */
    post: fn(T, &mut N, Nested) -> Vec<Effect>,   // `no_post` for a node with none
    held: Option<T>,                              // `Some` iff `pre`'s trigger matched
}

impl<'a, N, P, T> PathMut<'a, N, P, T> {
    /// Ascend one level, running the post at most once, and hand back its effects with the parent.
    /// `into_parent` consumes the level, so the value moves into the post and cannot be run twice.
    pub fn into_parent(mut self, nested: Nested) -> (Vec<Effect>, P) {
        let effs = self.held.take().map_or_else(Vec::new, |t| (self.post)(t, self.node_mut(), nested));
        (effs, self.parent)
    }
}

/// The default a node with no pre/post gets: `held` is `None`, so this returns no effects.
fn no_post<N, T>(_: T, _: &mut N, _: Nested) -> Vec<Effect> { Vec::new() }
```

`from_fn` takes the `post` and the held `Option<T>` — the child path is built with them, never mutated afterward. A pre/post node passes its `post` and `Some`/`None` from the descent-time trigger check; every other descent passes `no_post` and `None`. A node with several pre/posts stacks one such level per pre/post, each with its own `opt_i` and `post`, so each gets its own `into_parent` on the way up.

## The generated `Dispatch`

For `#[pre_post(AnyKey => (arm, stay))] Foo { #[resolve_into] bar: Bar }` — one pre/post. The `post` stored on the child path is the user's `stay` directly, no wrapper:

```rust
impl Dispatch<M> for Foo {
    fn dispatch<'a>(mut path: <Foo as Place>::Path<'a>, event: &M::Event)
        -> ControlFlow<(Vec<Effect>, Completed<'a>), (Vec<Effect>, <Foo as Place>::Path<'a>)>
    {
        // down: run the pre if its trigger matches, binding its return. `opt_0` is inferred from
        // `arm`'s return and never named. `None` records that the trigger missed, decided HERE.
        let opt_0 = match <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if AnyKey.is_matching(ev) => Some(arm(ev, ::bind::Node { parent: &mut path, data: () })),
            _ => None,
        };

        // descend, giving the child path Foo's `post` (the user's `stay`) and its held value
        let bar_path = ::laserbeam::PathMut::from_fn(
            path, |p| &mut p.get_mut().bar, |p| &p.get().bar, stay, opt_0,
        ).into();

        // up. (Foo has no exclusive bind of its own; a node that did would try it in the Continue
        // arm, below.)
        match <Bar as ::bind::Dispatch<M>>::dispatch(bar_path, event) {
            ::core::ops::ControlFlow::Break(won) => ::core::ops::ControlFlow::Break(won),
            ::core::ops::ControlFlow::Continue((below, bar_path)) => {
                let (mine, path) = bar_path.into_parent(::bind::Nested::Missed);
                let mut effs = below;
                ::core::iter::Extend::extend(&mut effs, mine);
                ::core::ops::ControlFlow::Continue((effs, path))
            }
        }
    }
}
```

A node WITH an exclusive bind tries it in the `Continue` arm, against the recovered path. The winner fires via `complete`, which ascends to root running the crossed posts, and the level returns `Break` so ancestors propagate without ascending again:

```rust
// inside the Continue arm, after the subtree missed and `path` was recovered. `effs` already holds
// the posts below this node.
if let Some(ev) = <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
    if Key::KeyN.down().is_matching(ev) {
        // to_nav returns ONLY its own effects plus the token; `complete` gathered the posts above
        let (own, mut done) = to_nav(ev, ::bind::Node { parent: path, data });
        ::core::iter::Extend::extend(&mut effs, done.take_gathered());   // upper posts (Handled)
        ::core::iter::Extend::extend(&mut effs, own);                    // then the winner
        return ::core::ops::ControlFlow::Break((effs, done));
    }
}
::core::ops::ControlFlow::Continue((effs, path))
```

## The guarantee

`pre` ran ⟹ `post` ran, exactly once. The post rides on the child path from construction, and there is exactly one ascent through that level — one `into_parent`, in the miss-unwind's `Continue` arm or inside `complete`. `into_parent` consumes the level, so the value moves into the post and cannot be run twice. The once-ness is the ownership, not a flag.

## The rearm as a user

`AndReturnHome { layers, guard }`, `#[pre_post(AnyKey => (arm, stay))]` plus the exclusive firing. `arm` returns the schedule; `stay` receives it as `T` and returns it:

```rust
// pre: mint a fresh timer, replace the old guard (cancelling the old timer), and RETURN the
// schedule. The guard stays on the node for its `Drop`; the schedule is the threaded `T`.
fn arm(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath, ()>) -> MercuryEffect {
    let (guard, schedule) = arm_return_home();
    node.parent.get_mut().guard = guard;
    schedule
}

// post: emit the schedule `arm` returned. `Nested` and the node unused here.
fn stay(schedule: MercuryEffect, _node: &mut AndReturnHome, _nested: Nested) -> Vec<MercuryEffect> {
    vec![schedule]
}
```

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over the existing tree:

- `pre` then `post` on a miss: an event the subtree does not bind runs `pre` (into `opt_0`), descends, misses, and the `Continue` arm's `into_parent` runs `post` with `Nested::Missed`; its effect lands in the returned `Vec`.
- `post` on a handled subtree: a leaf exclusive wins and `complete` climbs; `post` runs with `Nested::Handled` as `complete` crosses the pre/post node, and its effect lands in crossing order — after the posts below, before the winner's own.
- exactly-once: a drop counter on `pre`'s returned value shows it is consumed once, never twice and never zero, across both drivers.
- `pre` did not match: an event whose trigger `pre` rejects stores `None`, and `into_parent` runs no `post`.
- the exclusive winner is unchanged: leafward-most still wins, and cannot return without a `Completed`.

## Status

The work, prefactor first:

- prefactor, on master: `Bindings` drops `type Output` for `type Effect`; dispatch returns `Vec<M::Effect>`. `Dispatch`/`Descend` return `ControlFlow<(Vec<Effect>, Completed), (Vec<Effect>, Path)>`. laserbeam adds `Complete` (in the `AscendMut` family) and the sealed `Completed`; exclusive handlers return `(Vec<Effect>, Completed)` and reach root through `complete`, not `ascend_mut`. `into_parent` returns `(Vec<Effect>, Parent)`, empty until posts exist. Behavior-identical to master.
- feature: `PathMut` gains `post: fn(T, &mut N, Nested) -> Vec<Effect>`, `held: Option<T>`, and the `T` param; `into_parent` and `complete` run the posts. `bind_macro` gains `#[pre_post]` / `#[pre]` / `#[post]` — the descent-time trigger check binding `opt_i`, `from_fn` storing the user's `post` directly.

Open surface question: naming (`pre_post` vs two attributes; timing vs intent names).
