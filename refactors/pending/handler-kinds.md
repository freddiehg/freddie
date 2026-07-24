# pre/post handlers

Not done. Adds two handler positions to `bind` beside the existing exclusive one: `pre`, run on the way down and returning a value, and `post`, run on the way up with that value, guaranteed to run once if its `pre` ran. Dispatch becomes one descent and one leaf-to-root ascent (the sweep): the pres arm on the way down, the posts and the exclusive winner run on the way up, all folding into one `Vec<Effect>` threaded by ref.

## Two open decisions

Marked here because they set the shape of everything below and each contradicts an earlier call.

- Exclusive handler node type. This doc is written on the SYMMETRIC sweep: the winner does not consume the path, it reborrows the root through `root_mut`, so dispatch keeps owning the path and the sweep runs the posts above the winner. That forces the exclusive handler to take `Node<&mut P>` (borrowed, like a `pre`), not the owned `Node<P>` set earlier. Keeping owned `Node<P>` means the winner consumes to root (`ascend_mut`), which is the ASYMMETRIC version: `Break` carries no path, and every post runs before the winner instead of in crossing order.
- Effect order. Symmetric gives crossing order: the misses below the winner, the winner, the handled posts above it. Asymmetric gives all posts, then the winner. This doc assumes crossing order.

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
- `post`: `fn(T, &mut N, Nested, &mut Vec<Effect>)`. Runs on the way up when `pre` ran, taking `pre`'s return `T`, and pushes effects into the batch. Unlike an exclusive `#[bind]`, which returns effects the generated body extends into the batch, a `post` is a stored `fn` pointer run by generic `into_parent`, which cannot extend an opaque `impl IntoIterator` return through a pointer — so the `post` writes the `Vec` itself, and is stored as-is with no glue.

`pre`'s return survives the descent as an `Option<T>`: `Some(t)` when the trigger matched, `None` when it did not. The match is decided at the descent, up front — the `Option` records it — so `into_parent` runs `post` on the `Some` and skips the `None`, and nothing re-checks a trigger on the way up. `post` sees `T`, never the `Option`; the `Option` is only how the value is carried. `T` is inferred from `pre`'s return, never written (there is no name for it).

That `Option` is the one code path. The alternative — a match arm that calls `post` and one that does not, per pre/post — is `2^n` arms over a node. One `Option` per pre/post is `n` independent `Option`s and one path.

The two standalone forms are this same machine, not special cases:

- `#[pre]` alone: the `post` is `drop`. `pre` runs for its effect on the node, returns `T`, and `into_parent` calls `drop(t)`.
- `#[post]` alone: the `pre` is the trigger check, returning `()`. `Some(())` when it matched, `None` when it did not; `post` takes `()`.

`Nested` is the one outcome exposed — whether a handler won at or below this node — as an enum, so the meaning is named and the set can grow:

```rust
enum Nested {
    /// A handler won at or below this node (the sweep arm was Break).
    Handled,
    /// Nothing matched at or below this node (the sweep arm was Continue).
    Missed,
}
```

Nothing about the sweep mechanics is exposed — a `post` sees only `Nested`.

## The sweep: one descent, one ascent

Dispatch is one pass down and one pass up.

Down. Each node builds its child path (`from_fn`) and recurses. A pre/post node runs its `pre`s HERE, binding each to `opt_i: Option<T>` (`Some` iff the trigger matched) and storing them on the child path beside the posts. Nothing fires on the way down.

Up. The recursion unwinds leaf to root. At each level the node tries its own exclusive binds, deepest-first, unchanged from master. `into_parent(nested, out)` ascends one level and runs that level's post. The exclusive winner fires at its own level via `root_mut()` (reborrow the root, mutate, push effects) so the path survives and the sweep keeps running above it.

`out` is `&mut Vec<Effect>`, threaded through the whole pass. Effects land in crossing order: the misses below the winner, then the winner, then the handled posts above it.

Both control-flow arms ascend; the difference is `Nested` and whether the parent still tries its binds:

```rust
// Descend impl, implemented for the CHILD path. Both arms into_parent — symmetric.
match Child::Dispatch::dispatch(child_path, event, out) {
    // a winner was chosen below: ascend running the post as Handled, ancestors skip their binds
    Break(child_path)    => Break(child_path.into_parent(Nested::Handled, out)),
    // nothing below: ascend running the post as Missed, the parent will try its binds
    Continue(child_path) => Continue(child_path.into_parent(Nested::Missed, out)),
}
```

`Break` and `Continue` both carry the path now (on master `Break` carried the output). The path is what the sweep needs; the effects live in `out`.

## `PathMut`

`PathMut` carries the `post` and its held `Option<T>`. Before:

```rust
pub struct PathMut<'a, N, P> { /* projection to N, parent P */ }

impl<'a, N, P> HasParent for PathMut<'a, N, P> {
    type Parent = P;
    fn into_parent(self) -> P { /* project up */ }
}
```

after (`T` is `pre`'s return, inferred; the output is the concrete `Vec<Effect>`):

```rust
pub struct PathMut<'a, N, P, T> {
    /* projection to N, parent P */
    post: fn(T, &mut N, Nested, &mut Vec<Effect>),   // `no_post` for a node with none
    held: Option<T>,                                 // `Some` iff `pre`'s trigger matched
}

impl<'a, N, P, T> PathMut<'a, N, P, T> {
    /// Ascend one level, running the post at most once. `into_parent` consumes the level, so the
    /// value moves into the post and cannot be run twice. Runs only when `pre` matched.
    pub fn into_parent(mut self, nested: Nested, out: &mut Vec<Effect>) -> P {
        if let Some(t) = self.held.take() {
            (self.post)(t, self.node_mut(), nested, out);
        }
        self.parent
    }
}

/// The default a node with no pre/post gets: `held` is `None`, so this is never called.
fn no_post<N, T>(_: T, _: &mut N, _: Nested, _: &mut Vec<Effect>) {}
```

`from_fn` takes the `post` and the held `Option<T>` — the child path is built with them, never mutated afterward. A pre/post node passes its `post` and `Some`/`None` from the descent-time trigger check; every other descent passes `no_post` and `None`. A node with several pre/posts stacks one such level per pre/post, each with its own `opt_i` and `post`, so each gets its own `into_parent` on the way up.

## `root_mut`: the winner mutates without consuming

The exclusive winner reaches the root to mutate it. If it consumes the path to get there (`ascend_mut`), the path is gone and the sweep cannot run the posts above the winner. So laserbeam gains a non-consuming reborrow:

```rust
// walks the parent chain by &mut reborrow; the root path returns itself
fn root_mut(&mut self) -> &mut Root;
```

The winner does `node.parent.root_mut().set_layer(nav)`; the `&mut Root` dies at the end of that statement, so `node.parent` is free to be handed back in `Break`. Every mercury handler ascends to root today via `ascend_mut`, so each becomes `root_mut` and its node type flips from `Node<P>` to `Node<&mut P>` (see the open decisions).

## The generated `Dispatch`

For `#[pre_post(AnyKey => (arm, stay))] Foo { #[resolve_into] bar: Bar }` — one pre/post. The `post` stored on the child path is the user's `stay` directly, no wrapper:

```rust
impl Dispatch<M> for Foo {
    fn dispatch<'a>(mut path: <Foo as Place>::Path<'a>, event: &M::Event, out: &mut Vec<Effect>)
        -> ControlFlow<<Foo as Place>::Path<'a>, <Foo as Place>::Path<'a>>
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

        // up: both arms into_parent, running `stay` on the way past Foo. (Foo has no exclusive bind
        // of its own; a node that did would try it in the Continue arm, below.)
        match <Bar as ::bind::Dispatch<M>>::dispatch(bar_path, event, out) {
            ::core::ops::ControlFlow::Break(bar_path) =>
                ::core::ops::ControlFlow::Break(bar_path.into_parent(::bind::Nested::Handled, out)),
            ::core::ops::ControlFlow::Continue(bar_path) =>
                ::core::ops::ControlFlow::Continue(bar_path.into_parent(::bind::Nested::Missed, out)),
        }
    }
}
```

A node WITH an exclusive bind tries it in the `Continue` arm (nothing won below, so this level gets its turn). The winner fires via `root_mut`, pushes effects into `out`, and the level returns `Break` so ancestors skip their binds:

```rust
// inside the Continue arm, after the child subtree missed:
if let Some(ev) = <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
    if Key::KeyN.down().is_matching(ev) {
        // to_nav does `node.parent.root_mut().set_layer(nav)` and returns its effects
        Extend::extend(out, to_nav(ev, ::bind::Node { parent: &mut this_path, data }));
        return ::core::ops::ControlFlow::Break(this_path);   // path kept; the sweep runs posts above
    }
}
::core::ops::ControlFlow::Continue(this_path)
```

## The guarantee

`pre` ran ⟹ `post` ran, exactly once. The post rides on the child path from construction, and there is exactly one ascent through that level — one `into_parent`, in the `Break` arm or the `Continue` arm, both of which ascend. `into_parent` consumes the level, so the value moves into the post and cannot be run twice. The once-ness is the ownership, not a flag.

## The rearm as a user

`AndReturnHome { layers, guard }`, `#[pre_post(AnyKey => (arm, stay))]` plus the exclusive firing. `arm` returns the schedule; `stay` receives it as `T` and emits it:

```rust
// pre: mint a fresh timer, replace the old guard (cancelling the old timer), and RETURN the
// schedule. The guard stays on the node for its `Drop`; the schedule is the threaded `T`.
fn arm(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath, ()>) -> MercuryEffect {
    let (guard, schedule) = arm_return_home();
    node.parent.get_mut().guard = guard;
    schedule
}

// post: emit the schedule `arm` returned. `Nested` and the node unused here.
fn stay(schedule: MercuryEffect, _node: &mut AndReturnHome, _nested: Nested, out: &mut Vec<MercuryEffect>) {
    out.push(schedule);
}
```

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over the existing tree:

- `pre` then `post` on a miss: an event the subtree does not bind runs `pre` (into `opt_0`), descends, misses, and the `Continue` arm's `into_parent` runs `post` with `Nested::Missed`.
- `post` on a handled descent: a leaf exclusive wins and the sweep ascends; `post` runs with `Nested::Handled` in the `Break` arm as the sweep crosses the pre/post node, and its effect lands in crossing order — after the winner's, before an ancestor pre/post's.
- exactly-once: a drop counter on `pre`'s returned value shows it is consumed once, never twice and never zero, across both arms.
- `pre` did not match: an event whose trigger `pre` rejects stores `None`, and `into_parent` runs no `post`.
- the exclusive winner is unchanged: leafward-most still wins.

## Status

The work:

- `bind`: `Bindings` drops `type Output` and names `type Effect`; dispatch returns `Vec<M::Effect>`, seeded empty and threaded `&mut` so posts and the winner fold in order. `Dispatch`/`Descend` return `ControlFlow<Path, Path>` (was `ControlFlow<Output, Path>`); both arms `into_parent`.
- `laserbeam`: `PathMut` gains `post: fn(T, &mut N, Nested, &mut Vec<Effect>)`, `held: Option<T>`, and the `T` param; `into_parent` takes `(nested, out)`, unwraps `held`, runs `post` on `Some`. Adds `root_mut(&mut self) -> &mut Root`, a `&mut` reborrow up the parent chain.
- handlers: the exclusive handler stops consuming — `ascend_mut` becomes `root_mut`, and its node becomes `Node<&mut P>`. It still returns effects; the generated body extends `out`.
- `bind_macro`: the `#[pre_post]` / `#[pre]` / `#[post]` attributes and the generated body above — the descent-time trigger check binding `opt_i`, `from_fn` storing the user's `post` directly, and the symmetric `Break`/`Continue` arms.

Open surface question beyond the two decisions at the top: naming (`pre_post` vs two attributes; timing vs intent names).
