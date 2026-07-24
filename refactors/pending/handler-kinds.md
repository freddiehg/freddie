# pre/post handlers

Not done. Adds two handler positions to `bind` beside the existing exclusive one: `pre`, run on the way down, and `post`, run on the way up, guaranteed to run once if its `pre` ran. The mechanism is a `fn`-pointer stashed on the child path and run by `into_parent`.

## Syntax

Three attributes:

```rust
#[node(parent = FooParent)] #[binds(M)]
#[pre_post(AnyKey => (arm, stay))]   // the pair: `arm` stashes a value, `stay` reads it
struct Foo { #[resolve_into] bar: Bar }

#[pre(AnyKey => track)]    // pre alone: runs on the way down, stashes nothing
#[post(AnyKey => passthru)] // post alone: runs on the way up, reads nothing
```

`#[pre]` and `#[post]` are `#[pre_post]` with the other half absent and the threaded value `()`: a lone `#[pre]` registers no post, a lone `#[post]` runs with no stash. `#[pre_post]` is the only form that threads a real value from `pre` to `post` (through the node's stash field). A node may carry several, and the exclusive `#[bind]`, at once — their triggers being disjoint or not is the no-clobber question, unchanged for `#[bind]` and moot for the additive `pre`/`post`.

`trigger => handler` for the singles, `trigger => (pre, post)` for the pair. `pre`/`post` name the timing, not the intent, so a node reads better with the handlers named for what they do (`arm`/`stay`, `track`, `passthru`) and the attribute supplying the timing.

## What the two handlers are

Both handlers are `fn` items, not closures. That is the whole reason this is cheap: what gets stashed is a `fn` pointer, `Copy`, no `Box`, no `Rc`, no `dyn`.

- `pre`: `fn(&Event, Node<&mut P, D>)`. Runs on the descent if the trigger matches. Node borrowed (`&mut P`), so it can `get_mut` its own node and `ascend` to READ the root, but not `ascend_mut` to consume. It stashes whatever `post` needs in a field on its own node.
- `post`: `fn(&mut N, Nested) -> impl IntoIterator<Item = Effect>`. Runs on the way up. It reads what `pre` stashed in the node and returns effects. It gets `&mut N` (the node) and `Nested`.

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

`pre`'s output does not thread through the framework; `pre` stashes it in a field on its own node. The `post` is a `fn` pointer stashed on the CHILD path when it is constructed, and `into_parent` — the one funnel every ascent goes through, whether the normal unwind or a handler's `ascend_mut` — runs it. `Option::take` is the exactly-once.

`laserbeam`, `PathMut` gains one field and `into_parent` gains the post-running step. Before:

```rust
pub struct PathMut<'a, N, P> { /* projection to N, parent P */ }

impl<'a, N, P> HasParent for PathMut<'a, N, P> {
    type Parent = P;
    fn into_parent(self) -> P { /* project up */ }
}
```

after (`O` is the marker's `Output`, threaded so a run `post` can push its effects into the batch in order):

```rust
pub struct PathMut<'a, N, P, O> {
    /* projection to N, parent P */
    post: Option<fn(&mut N, Nested, &mut O)>,
}

impl<'a, N, P, O> PathMut<'a, N, P, O> {
    /// Descend and stash the post on the child in one step; `None` for a node with no pre/post.
    pub fn with_post(mut self, post: fn(&mut N, Nested, &mut O)) -> Self {
        self.post = Some(post);
        self
    }

    /// Ascend, running the stashed post exactly once. `nested` says what the descent did; `out`
    /// takes the post's effects, in order with the rest of the batch.
    pub fn into_parent(mut self, nested: Nested, out: &mut O) -> P {
        if let Some(post) = self.post.take() {
            post(self.node_mut(), nested, out);
        }
        self.parent
    }
}
```

`AscendMut::ascend_mut` gains the same `out: &mut O` parameter and threads it into each `into_parent` it walks through, passing `Nested::Handled` — that is how a handler climbing to the root runs the posts of the levels it crosses. The reflexive and per-depth `ascend_mut` impls all take and forward `out`.

## The generated `Dispatch`

For `#[pre_post(AnyKey => (arm, stay))] Foo { #[resolve_into] bar: Bar }`, the generated body — `pre` before the descent, the post stashed on the child, and the two exits both going through `into_parent`:

```rust
impl Dispatch<M> for Foo {
    fn dispatch<'a>(mut path: <Foo as Place>::Path<'a>, event: &M::Event, out: &mut M::Output)
        -> ControlFlow<(), <Foo as Place>::Path<'a>>
    {
        // pre: run if AnyKey matches; it stashes into the node. Path not consumed.
        if let Some(ev) = <&KeyEvent as TryFrom<_>>::try_from(event).ok() {
            if AnyKey.is_matching(ev) {
                arm(ev, ::bind::Node { parent: &mut path, data: () });
            }
        }

        // descend, stashing the post fn pointer on the child path
        let bar_path = ::laserbeam::PathMut::from_fn(path, |p| &mut p.get_mut().bar, |p| &p.get().bar)
            .into()
            .with_post(stay_glue);

        match <Bar as ::bind::Dispatch<M>>::dispatch(bar_path, event, out) {
            // A handler ascended THROUGH Foo — its `ascend_mut` ran `stay_glue` (Nested::Handled)
            // as it crossed this level. Nothing to do; the Break bubbles.
            ::core::ops::ControlFlow::Break(()) => ::core::ops::ControlFlow::Break(()),
            // bar missed — we ascend, running `stay_glue` (Nested::Missed).
            ::core::ops::ControlFlow::Continue(bar_path) => {
                let path = bar_path.into_parent(::bind::Nested::Missed, out);
                // (Foo's own exclusive binds would run here, then:)
                ::core::ops::ControlFlow::Continue(path)
            }
        }
    }
}

// Generated glue: a `fn` item (hence a plain `fn` pointer) adapting the user's `stay` — which
// returns effects — to the stashed `fn(&mut N, Nested, &mut O)` shape.
fn stay_glue(node: &mut Foo, nested: ::bind::Nested, out: &mut M::Output) {
    ::core::iter::Extend::extend(out, stay(node, nested));
}
```

## The guarantee

`pre` ran ⟹ `post` ran, exactly once. `pre` stashes the post on the child path. There is exactly one ascent through that level — either the framework's unwind (the `Continue` arm) or one handler's `ascend_mut` climbing past it — and that ascent calls `into_parent` once, which `take`s the post and runs it. `take` makes a second call (there is none) a no-op. No case matrix, no drop trickery: one stash, one funnel.

## The rearm as a user

`AndReturnHome { layers, guard, pending: Option<MercuryEffect> }`, `#[pre_post(AnyKey => (arm, stay))]` plus the exclusive firing:

```rust
// pre: mint a fresh timer, replace the old guard (cancelling it), stash the schedule unemitted.
fn arm(_ev: &KeyEvent, node: Node<&mut AndReturnHomePath, ()>) {
    let (guard, schedule) = arm_return_home();
    let arh = node.parent.get_mut();
    arh.guard = guard;
    arh.pending = Some(schedule);
}

// post: emit what `arm` stashed. `Nested` unused here.
fn stay(node: &mut AndReturnHome, _nested: Nested) -> Option<MercuryEffect> {
    node.pending.take()
}
```

Note this resets the timer on any key that reaches the wrapper, INCLUDING keys that then leave (`c`), whose `stay` runs during the ascent (before `set_layer`) and emits a schedule that the imminent layer swap cancels — the wasted arm. Distinguishing a stay from a leave needs to know the layer changed, which happens after the ascent, so it is a root fact; `pre/post` cannot see it, and the rearm either accepts the wasted arm or the reset stays at the root (`rearm_after`). `pre/post` is exact for effects whose decision does not depend on a state change that happens after the crossing; the rearm is a marginal fit, and that is a real input to whether to build this for the rearm specifically versus for a cleaner first user.

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over the existing tree:

- `pre` then `post` on a miss: an event the subtree does not bind runs `pre` (stashes), descends, misses, and `into_parent` runs `post` with `Nested::Missed`.
- `post` on a handled descent: a leaf exclusive fires and ascends; `post` runs with `Nested::Handled` as `ascend_mut` crosses the pre/post node, and its effects land in `out` in order before the exclusive winner's.
- exactly-once: a drop counter on the stashed value shows it is consumed once, never twice and never zero, across both arms.
- the exclusive winner is unchanged: leafward-most still wins.

## Status

The design is decided; the work is `laserbeam` (`PathMut` gains the `post` field and `O`; `into_parent`/`ascend_mut` thread `out` and run the stash) and `bind_macro` (the `#[pre_post]` attribute and the generated body above). The `fn`-pointer stash is what keeps it cheap. Naming (`pre_post` vs two attributes; timing vs intent names) is the one open surface question.
