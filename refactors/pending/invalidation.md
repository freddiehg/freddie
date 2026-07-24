# Invalidation: descent schedules, ascent executes

Not done. One doc for the whole pre/post/invalidation design. It supersedes `handler-kinds.md`, `exclusive-as-post.md`, and `dispatch-batch-and-complete.md` (all now in `past/`).

Dispatch is one pass down and one pass up. On the way DOWN, `pre`s read and reshape nothing, which fixes the set of handlers. On the way UP, `post`s run leaf to root, each handed a `Validity`, and a post acts only where its target still exists. A layer transition is scheduled by the deepest matcher on the descent and applied on the ascent by the level that owns the field; the posts below that field run after it, so a post learns it was invalidated.

## pre and post

Two handler positions, both `fn` items the user writes.

- `pre: fn(&Event, Node<&mut P, D>) -> (T, Vec<Effect>)`. Runs on the descent when the trigger matches. Borrowed node (`&mut P`): it can `get_mut` its own node and read an ancestor, not consume. It returns `(T, now-effects)`. The now-effects push as the descent enters the node. `T` is carried to this node's `post`.
- `post: fn(Validity<N>) -> Vec<Effect>`, given `pre`'s `T` when there is one. Runs on the ascent, once, iff its `pre` matched. It reads `Validity` (below) and returns effects.

`#[pre]` alone drops `T` (its post is `drop`). `#[post]` alone has a pre that is just the trigger check returning `()`. `#[pre_post]` is the only form that threads a real `T`. A node may carry several, at once.

```rust
#[pre_post(AnyKey => (arm, stay))]   // arm returns T, stay receives it
#[pre(AnyKey => track)]              // post is drop
#[post(AnyKey => guard)]             // pre is the trigger check
```

`pre`/`post` name the timing. The handlers are named for what they do (`arm`/`stay`, `guard`).

## Validity

A `post` at a node guards a CHILD field. After the node applies whatever reshape was scheduled for that field, it re-reads the field: still the guarded type gives `Valid`, replaced gives `Invalidated`.

```rust
struct Valid<'n, N> {
    node: &'n mut N,   // the guarded child, still present, reachable to mutate
    handled: bool,     // a handler matched at or below this field
}
enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated,       // the field is no longer an N
}
```

`Invalidated` carries nothing: there is no `N` to hand out, so touching a replaced node does not compile. `handled` records that a handler MATCHED below, not that state changed. Proving change needs tracking `get_mut` calls (possible, deferred), so `handled` stays conservative: it is true whenever any bound key fired below, whether or not the field moved.

`only_if_valid` runs a body on the valid side and drops the invalid one:

```rust
fn only_if_valid<N>(f: impl FnOnce(&mut N) -> Vec<Effect>) -> impl FnOnce(Validity<N>) -> Vec<Effect> {
    move |v| match v {
        Validity::Valid(valid) => f(valid.node),
        Validity::Invalidated => Vec::new(),
    }
}
```

It is a from-below signal. A post sees reshapes at or below its field (those ran first on the ascent); a reshape above it runs later and cannot reach effects it already returned.

## Why `&mut node` is sound

A post does not call `into_parent`; the framework does. Ascending one level:

```rust
pub fn into_parent(self, sink: &mut Vec<Effect>) -> P {
    // self owns the projection to this level's child field
    let v: Validity<N> = self.read_child();           // apply scheduled reshape, then classify
    Extend::extend(sink, (self.on_into_parent)(v));    // post borrows &mut N through Valid
    self.parent                                        // borrow ended; consume self, project up
}
```

The `&mut N` inside `Valid` is a reborrow scoped to the post call. `into_parent` still owns the path and consumes it to project up after the post returns. The two never overlap, so `Valid` handing a `&mut N` and `into_parent` needing ownership coexist.

## The sweep

Down. Each node builds its child path (`from_fn`) and recurses. A pre pushes its now-effects onto the batch and binds its `T` into `opt_i: Option<T>` (`Some` iff the trigger matched), captured by the child path's `on_into_parent` closure.

Up. The recursion unwinds leaf to root, threading one `effs: &mut Vec<Effect>`. Each level's post rides on its child's path as `on_into_parent`; `into_parent(sink)` runs it and pushes its effects onto `sink`, returning just the parent.

No handler ever holds a `&mut Vec<Effect>` or sees another handler's effects. Every handler RETURNS effects; the only holder of the batch is framework code (`into_parent`, `dispatch`). `from_fn`, the only way to build a child path and so the only way to choose an `on_into_parent`, is framework-only (crate-private). So a handler cannot smuggle in a closure that pops the batch. The capability is absent, not defended against.

`pre` ran ⟹ `post` ran, exactly once: the closure rides the child path from construction, there is one ascent through the level, and `into_parent` consumes the level and calls the `FnOnce` once. The once-ness is the ownership, not a flag.

```rust
pub struct PathMut<'a, N, P, F> {
    /* projection to N, parent P */
    on_into_parent: F,   // FnOnce(Validity<N>) -> Vec<Effect>; captures the pre values
}

fn no_post<N>(_: Validity<N>) -> Vec<Effect> { Vec::new() }   // the default for a node with no post
```

## Ordering

Effects land in nesting order: `A_pre, B_pre, <reshape>, B_post, A_post`. Pres on the way down, then the scheduled reshape applied at its owning level, then posts on the way up. A post runs AFTER the reshape that targets its field, so it can be told `Invalidated`. Running before would hide the reshape and re-arm a timer the transition just cancelled.

## The rearm

`AndReturnHome { inner: Layer, guard }` wraps a layer and holds a return-home timer. Its guard lives one level up, in the node that owns the `AndReturnHome` field, as a `#[post]` reading `Validity<AndReturnHome>`:

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;   // replaces the old guard, cancelling the old timer
    vec![schedule]
}
```

Navigate WITHIN (inner Home to Nav, `AndReturnHome` survives): `Valid`, rearm. Navigate OUT (the field is replaced by a bare layer): `Invalidated`, `only_if_valid` skips, and the dropped node's guard cancels the old timer. No wasted arm.

## The overlay

`OverlayLayer { shown, inner: Layer }` hides its overlay when a handler ran below. A `#[post]` at `OverlayLayer` reading `handled`, not a hide line in every navigation handler:

```rust
#[post(AnyKey => hide_on_change)]
fn hide_on_change(v: Validity<OverlayLayer>) -> Vec<MercuryEffect> {
    match v {
        Validity::Valid(valid) if valid.handled => vec![hide_overlay()],
        _ => vec![],
    }
}
```

It emits an effect and mutates no ancestor, so it needs `post` + `Validity` and nothing more.

## The prefactor (shippable alone, behavior-identical to master)

Independently reviewable and shippable before any pre/post exists. It threads effects as a batch and puts the `into_parent` seam in place; with no posts, nothing runs through it.

- `Bindings` drops `type Output` for `type Effect`. mercury's marker sets `type Effect = MercuryEffect`.
- `Dispatch`/`Descend` thread one `effs: &mut Vec<M::Effect>` and return `ControlFlow<(), Path>`. A matching bind pushes its effects onto `effs` and returns `Break(())`; a miss is `Continue(path)`.
- `into_parent` gains its sink parameter (`into_parent(self, sink)`); with no post it projects up and touches nothing.
- `bind::dispatch` seeds an empty `Vec`, returns `Some(effs)` on a win and `None` on a total miss (never `Some(vec![])`).
- A handler returns `V: Into<Vec<M::Effect>>`, so a single effect or a `Vec` both work. mercury adds `impl From<MercuryEffect> for Vec<MercuryEffect>`.

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}
```

Expression handlers already work: `#[bind(Keyboard("x") => plus(10))]` splices as `#handler(ev, node)`, so `plus(10)` is called and its result applied. Pinned by `crates/bind/tests/expr_handler.rs`. This is what an `only_if_valid(rearm)` handler position relies on.

## Open: how a transition reaches the owning level

The unresolved mechanism. A layer transition replaces `root.layer`, a field the deepest matcher does not own. Under the settled after-order the reshape has to be applied at the owning level BEFORE that level runs its posts. Candidate shapes:

- pre carries the reshape. The deep matcher's `pre` carries the target (a new layer, or a `FnOnce(&mut Owner)`) up as its `T`; the owning level's `post` applies it, then its guard posts run against the reshaped field. This folds the transition into the pre/post machine and drops the exclusive winner entirely. It needs a deepest-wins rule when two pres along one path both carry a reshape for the same field.
- a scheduler on the root. The descent records the reshape in a field the root owns; the ascent drains it at the owning level. This is the ambient-state shape the repo resists, and it is unjustified until a case needs a reshape that no single carried `T` can express.

`complete`/`ascend` (a deep winner consuming its path to mutate the root in place) is NOT a candidate: it runs the crossed posts before the reshape, which is the before-order this design rejected.

## Open: other

- `handled` is a bool. Precise change detection (track `get_mut`) and level-granular invalidation ("reshaped up to depth N") are deferred until a case needs them.
- Multiple children under one node: an enum (one active child) works with a single `Validity`; a product (several live children) needs one `Validity` per field and a join in `into_parent`.
- The single-owner claim: reaching a field mutably consumes the path, which consumes once, so at most one writer per field per dispatch. An additive second writer of a sibling field (`A { overlay, layer }`, one post writing `overlay` beside a transition writing `layer`) is allowed because the fields are disjoint; two writers of the SAME field is what single-owner forbids. Confirm this holds once the transition mechanism is chosen.
- Syntax: `#[pre_post]` plus `#[pre]`/`#[post]`, timing names vs intent names.
