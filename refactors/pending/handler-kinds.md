# pre/post handlers

Not done. Adds two handler positions to `bind` beside the existing exclusive one: `pre`, run on the way down and returning a value, and `post`, run on the way up with that value, guaranteed to run once if its `pre` ran. The mechanism is a `fn`-pointer and an `Option<T>` stored on the child path and run by `into_parent`.

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

Both handlers are `fn` items, not closures. That is the whole reason this is cheap: what gets stashed is a `fn` pointer, `Copy`, no `Box`, no `Rc`, no `dyn`.

- `pre`: `fn(&Event, Node<&mut P, D>) -> T`. Runs on the descent if the trigger matches. Node borrowed (`&mut P`), so it can `get_mut` its own node and `ascend` to READ the root, but not `ascend_mut` to consume. It RETURNS `T`.
- `post`: `fn(T, &mut N, Nested, &mut Output)`. Runs on the way up when `pre` ran, taking `pre`'s return `T`, and pushes effects into `Output`. Unlike an exclusive `#[bind]`, which returns effects the generated body extends into the batch, a `post` is a stored `fn` pointer run by generic `into_parent`, which cannot extend an opaque `impl IntoIterator` return through a pointer — so the `post` writes `Output` itself, and is stored as-is with no glue.

`pre`'s return survives the descent as an `Option<T>`: `Some(t)` when the trigger matched, `None` when it did not. The match is decided at the descent, up front — the `Option` records it — so `into_parent` runs `post` on the `Some` and skips the `None`, and nothing re-checks a trigger on the way up. `post` sees `T`, never the `Option`; the `Option` is only how the value is carried. `T` is inferred from `pre`'s return, never written (there is no name for it).

That `Option` is the one code path. The alternative — a match arm that calls `post` and one that does not, per pre/post — is `2^n` arms over a node. One `Option` per pre/post is `n` independent `Option`s and one path.

The two standalone forms are this same machine, not special cases:

- `#[pre]` alone: the `post` is `drop`. `pre` runs for its effect on the node, returns `T`, and `into_parent` calls `drop(t)`.
- `#[post]` alone: the `pre` is the trigger check, returning `()`. `Some(())` when it matched, `None` when it did not; `post` takes `()`.

`Nested` is the one outcome exposed — whether the descent below this node matched a handler — as an enum, so the meaning is named and the set can grow:

```rust
enum Nested {
    /// A handler ran at or below this node (the descent Broke).
    Handled,
    /// Nothing matched below this node (the descent missed).
    Missed,
}
```

Nothing about the ascent mechanics is exposed — a `post` cannot tell whether it is being run by the normal unwind or by a handler climbing past it, only `Nested`.

## The mechanism: stash on the path, run in `into_parent`

Each `pre` returns a value the generated body binds to a local; a node's pre values are carried, as `Option`s, on the CHILD path when it is constructed, beside a single generated `post` `fn` that runs them. `into_parent` — the one funnel every ascent goes through, the normal unwind or a handler's `ascend_mut` — runs that `fn`, once, because it consumes the level.

`PathMut` carries the `post` and its held `Option<T>`. Before:

```rust
pub struct PathMut<'a, N, P> { /* projection to N, parent P */ }

impl<'a, N, P> HasParent for PathMut<'a, N, P> {
    type Parent = P;
    fn into_parent(self) -> P { /* project up */ }
}
```

after (`O` is the marker's `Output`, threaded so a run post can push effects into the batch in order; `T` is `pre`'s return, inferred):

```rust
pub struct PathMut<'a, N, P, O, T> {
    /* projection to N, parent P */
    post: fn(T, &mut N, Nested, &mut O),   // `no_post` for a node with none
    held: Option<T>,                       // `Some` iff `pre`'s trigger matched; `None` otherwise
}

impl<'a, N, P, O, T> PathMut<'a, N, P, O, T> {
    /// Ascend, running the post at most once — `into_parent` consumes the level, so the value moves
    /// into the post and cannot be run twice. Runs only when `pre` matched (`held` is `Some`).
    pub fn into_parent(mut self, nested: Nested, out: &mut O) -> P {
        if let Some(t) = self.held.take() {
            (self.post)(t, self.node_mut(), nested, out);
        }
        self.parent
    }
}

/// The default a node with no pre/post gets: `held` is `None`, so this is never called.
fn no_post<N, O, T>(_: T, _: &mut N, _: Nested, _: &mut O) {}
```

`from_fn` takes the `post` and the held `Option<T>` — the child path is built with them, never mutated afterward. A pre/post node passes its `post` and `Some`/`None` from the descent-time trigger check; every other descent passes `no_post` and `None`.

`AscendMut::ascend_mut` gains the same `out: &mut O` parameter and threads it into each `into_parent` it walks through, passing `Nested::Handled` — that is how a handler climbing to the root runs the posts of the levels it crosses. The reflexive and per-depth `ascend_mut` impls all take and forward it.

A node with several pre/posts is several levels' worth of this, one `(post, Option<T>)` per pre/post; the generated descent chains them so each gets its own `into_parent`.

## The generated `Dispatch`

For `#[pre_post(AnyKey => (arm, stay))] Foo { #[resolve_into] bar: Bar }` — one pre/post. The `post` stored on the child path is the user's `stay` directly, no wrapper:

```rust
impl Dispatch<M> for Foo {
    fn dispatch<'a>(mut path: <Foo as Place>::Path<'a>, event: &M::Event, out: &mut M::Output)
        -> ControlFlow<(), <Foo as Place>::Path<'a>>
    {
        // pre: run it if its trigger matches, binding its return. `opt_0` is Foo's first (only)
        // pre/post; its type is inferred from `arm`'s return and never named. `None` records that
        // `arm`'s trigger did not match this event — that decision is made HERE, up front, so the
        // ascent runs `stay` on the `Some` and re-checks nothing.
        let opt_0 = match <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if AnyKey.is_matching(ev) => Some(arm(ev, ::bind::Node { parent: &mut path, data: () })),
            _ => None,
        };

        // descend, giving the child path Foo's `post` (the user's `stay`) and its held value
        let bar_path = ::laserbeam::PathMut::from_fn(
            path, |p| &mut p.get_mut().bar, |p| &p.get().bar, stay, opt_0,
        ).into();

        match <Bar as ::bind::Dispatch<M>>::dispatch(bar_path, event, out) {
            // a handler ascended THROUGH Foo — its `ascend_mut` ran `stay` (Nested::Handled), but
            // only if `opt_0` was `Some`
            ::core::ops::ControlFlow::Break(()) => ::core::ops::ControlFlow::Break(()),
            // bar missed — we ascend, running `stay` (Nested::Missed) if `opt_0` was `Some`
            ::core::ops::ControlFlow::Continue(bar_path) =>
                ::core::ops::ControlFlow::Continue(bar_path.into_parent(::bind::Nested::Missed, out)),
        }
    }
}
```

`stay` is stored as-is: it already has the `post` shape `fn(T, &mut N, Nested, &mut O)`. `into_parent` unwraps `opt_0` and, on `Some(t)`, calls `stay(t, node, nested, out)`. A node with several pre/posts stacks one such level per pre/post, each with its own `opt_i` and `post`, so each still stores the user's fn directly and there is no generated wrapper.

## The guarantee

`pre` ran ⟹ `post` ran, exactly once. The post rides on the child path from construction. There is exactly one ascent through that level — either the framework's unwind (the `Continue` arm) or one handler's `ascend_mut` climbing past it — and that ascent is one `into_parent`, which consumes the level and runs the post. You cannot ascend through a moved-out level twice, so the once-ness is the ownership, not any flag. No case matrix, no drop trickery: one post per level, one funnel.

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
fn stay(schedule: MercuryEffect, _node: &mut AndReturnHome, _nested: Nested, out: &mut Output) {
    Extend::extend(out, [schedule]);
}
```

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over the existing tree:

- `pre` then `post` on a miss: an event the subtree does not bind runs `pre` (returns its value into `opt_0`), descends, misses, and `into_parent` runs `post` with `Nested::Missed`.
- `post` on a handled descent: a leaf exclusive fires and ascends; `post` runs with `Nested::Handled` as `ascend_mut` crosses the pre/post node, and its effects land in `out` in order before the exclusive winner's.
- exactly-once: a drop counter on `pre`'s returned value shows it is consumed once, never twice and never zero, across both arms.
- `pre` did not match: an event whose trigger `pre` rejects stores `None`, and `into_parent` runs no `post`.
- the exclusive winner is unchanged: leafward-most still wins.

## Status

The work:

- `bind`: `Dispatch::dispatch` and `Descend::dispatch` thread `out: &mut Output` by-ref and return `ControlFlow<(), Path>` (was `ControlFlow<Output, Path>`). Effects accumulate into `out` in ascent order; `Break(())` means a handler fired, `Continue(path)` a miss.
- `laserbeam`: `PathMut` gains `post: fn(T, &mut N, Nested, &mut O)`, `held: Option<T>`, and the `O`/`T` params. `into_parent` takes `(nested, out)`, unwraps `held`, and runs `post` on `Some`. `ascend_mut` takes `out`, passes `Nested::Handled` into each `into_parent` it crosses, so a firing handler's climb runs the crossed posts.
- handlers: a firing handler hands `out` to `ascend_mut`, so its signature gains `out`. The fired handler's own effects land after the crossed posts.
- `bind_macro`: the `#[pre_post]` / `#[pre]` / `#[post]` attributes and the generated body above — the descent-time trigger check binding `opt_i`, and `from_fn` storing the user's `post` fn directly.

The `fn`-pointer post is what keeps it cheap. Naming (`pre_post` vs two attributes; timing vs intent names) is the one open surface question.
