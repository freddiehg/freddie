# Invalidation: descent schedules, ascent executes

Not done. Standalone. Pre, post, exclusive, and Validity are one machine. The generate for one tree is the design.

Dispatch is one pass down and one pass up. On the way DOWN, `pre`s read and reshape nothing, which freezes the set of handlers. On the way UP, `post`s run leaf to root, each handed a `Validity`, and a post acts only where its target still exists. A reshape is applied on the ascent at the level that owns the field, before that level's posts, so a post learns it was invalidated.

## The case

```rust
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[bind(a => outer_handler)]
struct Outer {
    #[resolve_into]
    inner: Inner,
}

#[bind(a => inner_handler)]
struct Inner;
```

Two `pre_post` pairs on Outer, exclusive `a` on both levels. Deepest exclusive wins. Outer posts run on every ascent that entered Outer, including when Inner claimed `a`.

## pre and post

Two handler positions, both `fn` items the user writes. What rides the path is a monomorphized `FnOnce` closure `on_into_parent` that captured the node's `opt_i`s — no `Box`, no `Rc`, no `dyn`.

- `pre: fn(&Event, Node<&mut P, D>) -> (T, Vec<Effect>)`. Runs on the descent when the trigger matches. Borrowed node (`&mut P`): `get_mut` its own node, read an ancestor, not consume. Returns `(T, now-effects)`. Now-effects push as the descent enters the node. `T` is carried to this node's `post`.
- `post: fn(T, Validity<'_, Child>) -> Vec<Effect>`. Runs on the ascent, once, iff its `pre` matched. Reads `Validity` and returns effects. `T = ()` when there is no real pre.

`#[pre]` alone: post is `drop`. `#[post]` alone: pre is the trigger check returning `()`. `#[pre_post]` is the only form that threads a real `T`. `#[bind]` is exclusive (below). A node may carry several at once.

```rust
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[pre(AnyKey => track)]              // post is drop
#[post(AnyKey => guard)]             // pre is the trigger check
#[bind(a => outer_handler)]          // exclusive post
```

`pre`/`post` name the timing. Handlers are named for what they do (`pre_foo`/`post_foo`, `rearm`, `outer_handler`).

`pre`'s return survives the descent as `Option<T>`: `Some(t)` when the trigger matched, `None` when it did not. The match is decided on the descent — the `Option` records it — so `into_parent` runs `post` on `Some` and skips `None`, and nothing re-checks a trigger on the way up. `post` sees `T`, never the `Option`. `T` is inferred from `pre`'s return, never written.

That `Option` is the one code path. The alternative — a match arm that calls `post` and one that does not, per pre/post — is `2^n` arms over a node. One `Option` per pre/post is `n` independent `Option`s and one path.

## Exclusive is a post

```rust
#[bind(a => handler)]
// lowers to:
#[post(a => exclusive(handler))]
```

The exclusive body runs only when nothing deeper already claimed the event (`!handled`). It sets `handled` for ancestors. Deepest-wins is that gate plus leaf-to-root order. There is no separate exclusive search on the unwind, and no short-circuit past parents: every level always ascends through `into_parent`, so posts always run.

```rust
// exclusive(handler) expands to, on the way up when the trigger matched:
if !*handled {
    *handled = true;
    handler(ev, node)   // Node<&mut P, D> — path stays with the framework
} else {
    vec![]
}
```

Trigger match is decided on the descent (binding `opt` the closure captures, same as every other post). Matched is what makes the body eligible; `!handled` is what makes it win.

Exclusive and pre both take `Node<&mut P, D>` (borrowed path). The framework keeps the path so ascent finishes after a deep exclusive. That is a change from today's exclusive `Node<P>` by value.

An exclusive that reshapes a field must not apply the write mid-ascent if shallower posts still need that field. Reshape is scheduled on the descent (or as the exclusive's carried action) and applied in the owning level's `into_parent` before that level's posts. Carrier: Open at the end.

## Validity

A `post` at a node guards a CHILD field. After the node applies whatever reshape was scheduled for that field, it re-reads the field: still the guarded type gives `Valid`, replaced gives `Invalidated`.

```rust
struct Valid<'n, N> {
    node: &'n mut N,   // the guarded child, still present, reachable to mutate
    handled: bool,     // an exclusive claimed at or below this field
}
enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated,       // the field is no longer an N
}
```

`Invalidated` carries nothing: there is no `N` to hand out, so touching a replaced node does not compile. `handled` records that an exclusive CLAIMED below, not that state changed. Proving change needs tracking `get_mut` calls (possible, deferred), so `handled` stays a claim bit.

`only_if_valid` runs a body on the valid side and drops the invalid one:

```rust
fn only_if_valid<N>(
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Validity<'_, N>) -> Vec<Effect> {
    move |v| match v {
        Validity::Valid(valid) => f(valid.node),
        Validity::Invalidated => Vec::new(),
    }
}
```

It is a from-below signal. A post sees reshapes at or below its field (those ran first on the ascent); a reshape above it runs later and cannot reach effects it already returned. Effects that already left a post as an owned `Vec` survive a shallower reshape.

## Why `&mut node` is sound

A post does not call `into_parent`; the framework does. Ascending one level:

```rust
pub fn into_parent(self, handled: bool, sink: &mut Vec<Effect>) -> P {
    // self owns the projection to this level's child field
    let v: Validity<'_, N> = self.read_child(handled); // apply scheduled reshape, then classify
    Extend::extend(sink, (self.on_into_parent)(v));    // post borrows &mut N through Valid
    self.parent                                        // borrow ended; consume self, project up
}
```

The `&mut N` inside `Valid` is a reborrow scoped to the post call. `into_parent` still owns the path and consumes it to project up after the post returns. The two never overlap, so `Valid` handing a `&mut N` and `into_parent` needing ownership coexist.

## Path and batch

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub struct PathMut<N, P, F> {
    /* projection to N, parent P */
    on_into_parent: F, // FnOnce(Validity<'_, N>) -> Vec<Effect>; captures the pre values
}

fn no_post<N>(_: Validity<'_, N>) -> Vec<Effect> {
    Vec::new()
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        handled: &mut bool,
    ) -> Self::Path<'a>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let mut handled = false;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut handled);
    if handled || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

No handler ever holds a `&mut Vec<Effect>` or sees another handler's effects. Every handler RETURNS effects; the only holder of the batch is framework code (`into_parent`, `dispatch`). `from_fn`, the only way to build a child path and so the only way to choose an `on_into_parent`, is framework-only (crate-private). A handler cannot smuggle in a closure that pops the batch. The capability is absent, not defended against.

`pre` ran ⇒ `post` ran, exactly once: the closure rides the child path from construction, there is one ascent through the level, and `into_parent` consumes the level and calls the `FnOnce` once. The once-ness is the ownership, not a flag.

A node with several pre/posts captures every `opt_i` in ONE closure, which runs each post — one closure per node, no stack of levels, no tuple.

## Ordering

```text
enter Outer
  pre_foo? pre_bar?                 // now-effects, bind opt_*
  opt_a = exclusive trigger check   // body later
  build inner_path                  // Outer posts closed over opts
  enter Inner
    exclusive inner_handler?        // may set handled
  leave Inner
  into_parent(handled)              // reshape .inner?, Validity, post_foo?, post_bar?
  exclusive outer_handler?          // only if !handled
leave Outer
```

Effects land in nesting order: pres on the way down, then exclusive at depth, then the scheduled reshape at its owning level, then posts observing that field, then shallower exclusives. A post runs AFTER the reshape that targets its field, so it can be told `Invalidated`. Running before would hide the reshape and re-arm a timer the transition just cancelled.

## Generated `Inner`

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        mut path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        handled: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        // #[bind(a => inner_handler)] → exclusive post
        if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
            if a.is_matching(ev) && !*handled {
                *handled = true;
                Extend::extend(
                    effs,
                    Into::<Vec<M::Effect>>::into(inner_handler(
                        ev,
                        ::bind::Node {
                            parent: &mut path,
                            data: (),
                        },
                    )),
                );
            }
        }
        path
    }
}
```

## Generated `Outer`

```rust
impl Dispatch<M> for Outer {
    fn dispatch<'a>(
        mut path: <Outer as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        handled: &mut bool,
    ) -> <Outer as Place>::Path<'a>
    where
        Self: 'a,
    {
        // ----- down: each pre_post independently -----
        let opt_foo = match <&FooEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Foo.is_matching(ev) => {
                let (t, now) = pre_foo(
                    ev,
                    ::bind::Node {
                        parent: &mut path,
                        data: (),
                    },
                );
                Extend::extend(effs, now);
                Some(t)
            }
            _ => None,
        };
        let opt_bar = match <&BarEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Bar.is_matching(ev) => {
                let (t, now) = pre_bar(
                    ev,
                    ::bind::Node {
                        parent: &mut path,
                        data: (),
                    },
                );
                Extend::extend(effs, now);
                Some(t)
            }
            _ => None,
        };
        // exclusive: trigger check only on the way down
        let opt_a = match <&AEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if a.is_matching(ev) => Some(ev),
            _ => None,
        };

        // ----- descend: one on_into_parent captures every opt_i -----
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |v| {
                let mut local = ::std::vec::Vec::new();
                match v {
                    Validity::Valid(mut valid) => {
                        if let Some(t) = opt_foo {
                            Extend::extend(
                                &mut local,
                                post_foo(
                                    t,
                                    Validity::Valid(Valid {
                                        node: &mut *valid.node,
                                        handled: valid.handled,
                                    }),
                                ),
                            );
                        }
                        if let Some(t) = opt_bar {
                            Extend::extend(
                                &mut local,
                                post_bar(
                                    t,
                                    Validity::Valid(Valid {
                                        node: &mut *valid.node,
                                        handled: valid.handled,
                                    }),
                                ),
                            );
                        }
                    }
                    Validity::Invalidated => {
                        if let Some(t) = opt_foo {
                            Extend::extend(&mut local, post_foo(t, Validity::Invalidated));
                        }
                        if let Some(t) = opt_bar {
                            Extend::extend(&mut local, post_bar(t, Validity::Invalidated));
                        }
                    }
                }
                local
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, handled);

        // ----- up: reshape .inner?, Outer posts, then Outer exclusive -----
        let mut path = inner_path.into_parent(*handled, effs);

        if let Some(ev) = opt_a {
            if !*handled {
                *handled = true;
                Extend::extend(
                    effs,
                    Into::<Vec<M::Effect>>::into(outer_handler(
                        ev,
                        ::bind::Node {
                            parent: &mut path,
                            data: (),
                        },
                    )),
                );
            }
        }
        path
    }
}
```

## Walks

### `a` only

```text
opt_foo None, opt_bar None, opt_a Some
inner_handler runs, handled = true
into_parent: Valid { handled: true }, post_foo skip, post_bar skip
outer_handler skip
```

Batch: `inner_handler` only. Outer posts still ran (no-op).

### Foo only

```text
pre_foo runs, opt_foo = Some(t)
Inner exclusive skip
into_parent: Valid { handled: false }, post_foo(t, Valid), post_bar skip
outer exclusive skip
```

Batch: `pre_foo` now-effects, then `post_foo`.

### Foo and `a`

```text
pre_foo runs
inner_handler runs, handled = true
into_parent: Valid { handled: true }, post_foo(t, Valid { handled: true })
outer_handler skip
```

Batch: `pre_foo` now-effects, `inner_handler`, `post_foo`.

### Inner exclusive schedules reshape of `.inner`

```text
inner_handler claims and schedules reshape
into_parent: apply reshape → Invalidated
post_foo(t, Invalidated) if Foo matched
outer exclusive skip
```

Posts always run when their pre matched. They decide what `Invalidated` means.

## The rearm

`AndReturnHome { layers, guard }` wraps timed layers and holds a return-home timer. The rearm lives one level up, in the node that owns the `AndReturnHome` field, as a `#[post]` reading `Validity<AndReturnHome>`:

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;   // replaces the old guard, cancelling the old timer
    vec![schedule]
}
```

Navigate WITHIN (inner Nav to Site, `AndReturnHome` survives): `Valid`, rearm. Navigate OUT (the field is replaced by a bare layer): `Invalidated`, `only_if_valid` skips, and the dropped node's guard cancels the old timer. No wasted arm. Mint is on the valid ascent only — no `arm` pre, no threaded schedule, no `pending` slot.

## The overlay

`OverlayLayer { shown, inner }` hides its overlay when a handler claimed below. A `#[post]` at `OverlayLayer` reading `handled`, not a hide line in every navigation handler:

```rust
#[post(AnyKey => hide_on_change)]
fn hide_on_change(v: Validity<'_, OverlayLayer>) -> Vec<MercuryEffect> {
    match v {
        Validity::Valid(valid) if valid.handled => vec![hide_overlay()],
        _ => vec![],
    }
}
```

It emits an effect and mutates no ancestor, so it needs `post` + `Validity` and nothing more.

## Single-owner

Reaching a field mutably consumes the path once, so at most one writer per field per dispatch. An additive second writer of a sibling field (`A { overlay, layer }`, one post writing `overlay` beside a transition writing `layer`) is allowed because the fields are disjoint. Two writers of the SAME field is what single-owner forbids. A non-winner that must mutate root state that cannot relocate to its own node would need a re-derivable-path scheduler; no mercury case needs that yet.

## Effects survive invalidation

A post that runs before a shallower reshape keeps its effects. In `root → layer → NavLayer`, the deep post returns `fx`, then a shallower level swaps the layer; `fx` stays, because it is an owned `Vec<Effect>` the later swap cannot reach into.

## The prefactor

Independently reviewable and shippable before any pre/post exists. Threads the batch and puts the `into_parent` seam in place; with no posts, nothing runs through it. Behavior-identical exclusive dispatch for today's tree once exclusives borrow the path.

Before (`crates/bind/src/lib.rs`):

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Output;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
    ) -> ControlFlow<M::Output, Self::Path<'a>>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<M::Output>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    match <N as Dispatch<M>>::dispatch(path, event) {
        ControlFlow::Break(out) => Some(out),
        ControlFlow::Continue(_) => None,
    }
}
```

After:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        handled: &mut bool,
    ) -> Self::Path<'a>
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let mut handled = false;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut handled);
    if handled || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

- mercury's marker sets `type Effect = MercuryEffect` where it set `type Output = Vec<MercuryEffect>`
- exclusives take `Node<&mut P, D>`, set `handled`, push effects via `V: Into<Vec<Effect>>`, return path
- `into_parent(self, _handled, _sink)` projects up; with no post it touches nothing
- `from_fn` is crate-private
- mercury adds `impl From<MercuryEffect> for Vec<MercuryEffect>`

Expression handlers already work: `#[bind(Keyboard("x") => plus(10))]` splices as `#handler(ev, node)`, so `plus(10)` is called and its result applied. Pinned by `crates/bind/tests/expr_handler.rs`. That is what an `only_if_valid(rearm)` handler position relies on.

Before/after a mercury exclusive (`to_nav`):

```rust
// before
pub(crate) fn to_nav<'a, E, P: AscendMut<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend_mut().set_layer(nav);
    effects.push(timer);
    effects
}

// after — path borrowed; ascend_mut still available through &mut if the path type allows,
// or the reshape is scheduled (Open) and applied at the owner
pub(crate) fn to_nav<'a, E, P>(
    _ev: &E,
    node: Node<&mut P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend_mut().set_layer(nav);
    effects.push(timer);
    effects
}
```

## Rules the generate encodes

1. Several `pre_post`s on one node → several `opt_i`, one `on_into_parent` closure.
2. `pre` matched ⇒ `post` ran exactly once (`FnOnce` on the path, one ascent).
3. Posts run only inside `into_parent`. Claim does not skip them.
4. Deepest exclusive wins via `handled`.
5. Exclusive and pre take `Node<&mut P, D>`. Path stays with the framework.
6. Reshape of a field is applied in that field's `into_parent` before `Validity` is built.

## Tests

`crates/bind/tests/`, a `#[pre_post]` node over a two-level tree matching Outer/Inner:

- `pre` then `post` on a miss: event the subtree does not bind runs `pre`, descends, misses, `into_parent` runs `post` with `Valid { handled: false }`
- `post` on a claimed subtree: leaf exclusive wins, parent `into_parent` runs `post` with `Valid { handled: true }`; batch is `A_pre, winner, A_post`
- exactly-once: a drop counter on `pre`'s returned value shows it is consumed once, never twice and never zero
- `pre` did not match: `opt` is `None`, `into_parent` runs no `post`
- deepest exclusive wins: both levels bind `a`, only the leaf runs
- parent exclusive runs when the leaf misses: leaf has no bind for the event, parent exclusive fires after `into_parent`
- reshape → `Invalidated`: scheduled replace of the child field, post receives `Invalidated` and must not be handed an `N`
- expression handler position: `only_if_valid(rearm)` form, pinned beside `expr_handler.rs`

`crates/mercury/tests/transitions.rs` keeps the same effect assertions once rearm moves off `handle`'s discriminant check onto the post.

## Open: how a transition reaches the owning level

A layer transition replaces `root.layer`, a field the deepest matcher does not own. Under the settled after-order the reshape is applied at the owning level BEFORE that level runs its posts. Candidate shapes:

- pre/exclusive carries the reshape. The deep matcher's carried value is the target (a new layer, or a `FnOnce(&mut Owner)`); the owning level applies it in `into_parent`, then guard posts run against the reshaped field. Needs a deepest-wins rule when two along one path both carry a reshape for the same field (`handled` already provides deepest exclusive).
- a scheduler on the root. The descent records the reshape in a field the root owns; the ascent drains it at the owning level. Ambient state the program mints; unjustified until a case needs a reshape that no single carried value can express.

Whichever wins, the reshape is applied at the owning level on the ascent, before that level's posts run. A post sees the field after the reshape (`Valid` or `Invalidated`).

## Open: other

- `handled` is a bool. Precise change detection (track `get_mut`) and level-granular invalidation ("reshaped up to depth N") are deferred until a case needs them.
- Multiple children under one node: an enum (one active child) works with a single `Validity`; a product (several live children) needs one `Validity` per field and a join in `into_parent`.
- Confirm single-owner once the transition carrier is chosen: sibling-field writers remain fine; same-field double writers remain forbidden.
- Syntax: `#[pre_post]` plus `#[pre]`/`#[post]`, timing names vs intent names — attributes stay timing names; handlers stay intent names.
