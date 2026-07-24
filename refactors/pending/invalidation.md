# Invalidation: descent schedules, ascent executes

Not done. Standalone. The generate for one tree is the design.

Dispatch walks down, then up. On the way down, matching `pre`s run (read-only) and stash a value in an `Option`. On the way up, matching `post`s run with that value, a mutable path, and whether the subtree was invalidated / handled. A reshape of a field is applied at the owning level on the way up, before that level's posts, so a post can see `Invalidated`.

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

## Semantics

### pre

```rust
// immutable access only — no get_mut, no reshape
fn pre_foo(ev: &FooEvent, node: Node<&OuterPath, ()>) -> T
```

- Runs on the descent when the trigger matches.
- Gets an **immutable** reference to the node/path (`Node<&P, D>` / shared path).
- Returns arbitrary data `T`. The framework stores it in `opt: Option<T>` — `Some(t)` if the trigger matched, `None` otherwise.
- The match is decided here; the ascent does not re-check the trigger.
- Does not reshape. That freezes the handler set for the rest of the walk.

Open: whether `pre` may also push now-effects (`-> (T, Vec<Effect>)`). No mercury case needs them yet; the generate below is `-> T` only. Add the pair if a pre must emit on the way down.

### post

```rust
fn post_foo(t: T, path: &mut OuterPath, v: Validity<'_, Inner>) -> Vec<Effect>
```

- Runs on the ascent, **once**, iff its pre matched (`opt` is `Some`).
- Gets:
  - `t: T` — the value the pre returned (moved out of the `Option`)
  - `path: &mut Path` — mutable path at this node
  - `v: Validity` — whether the guarded child field still exists, and whether something below handled the event
- Returns effects. Framework pushes them onto the batch.

```rust
struct Valid<'n, N> {
    node: &'n mut N,   // child field still this type; mutably reachable
    handled: bool,     // exclusive claimed at or below this field
}
enum Validity<'n, N> {
    Valid(Valid<'n, N>),
    Invalidated,       // child field is no longer an N — no node to hand out
}
```

`Invalidated` carries nothing: a replaced node is not reachable. `handled` is claim of the event below, not "state changed."

### Defaults when one half is missing

| attribute | pre | post |
|---|---|---|
| `#[pre_post(trig => (pre, post))]` | user `pre` → `T` | user `post(t, path, v)` |
| `#[pre(trig => pre)]` | user `pre` → `T` | **drop** `t` (no effects) |
| `#[post(trig => post)]` | trigger check → `()` | user `post((), path, v)` |
| `#[bind(trig => handler)]` | trigger check → `()` | exclusive body (below) |

- No pre provided → `T = ()`, pre is just the trigger check.
- No post provided → the stashed `T` is dropped on the way up at the point the post would have run.
- Several pre/posts on one node → several independent `opt_i: Option<Ti>`, one code path (not `2^n` arms).

### `#[bind]` is one function

`#[bind(trig => handler)]` is the logical combination of a pre and a post in **one** user function:

- **pre half** (framework): trigger matches → `opt = Some(())`
- **post half** (user): one function, runs on the way up when `opt` is `Some` and nothing deeper has claimed (`!handled`)

```rust
// user writes one function:
fn outer_handler(ev: &AEvent, path: &mut OuterPath, v: Validity<'_, Inner>) -> Vec<Effect>

// framework treats it as exclusive post:
//   if opt_a.is_some() && !handled {
//       handled = true;
//       effs.extend(outer_handler(ev, path, v));
//   }
```

Deepest-wins: the deepest matching bind sees `handled == false`, runs, sets `handled = true`; shallower binds see `true` and skip. No short-circuit past parents — every level still ascends so other posts run.

`only_if_valid` is sugar for posts that only act on the valid side:

```rust
fn only_if_valid<N, P>(
    f: impl FnOnce(&mut P, &mut N) -> Vec<Effect>,
) -> impl FnOnce(&mut P, Validity<'_, N>) -> Vec<Effect> {
    move |path, v| match v {
        Validity::Valid(valid) => f(path, valid.node),
        Validity::Invalidated => Vec::new(),
    }
}
```

## Walk (down, then up)

```text
enter Outer
  if Foo matches:  opt_foo = Some(pre_foo(ev, &path))   // immutable
  if Bar matches:  opt_bar = Some(pre_bar(ev, &path))
  if a matches:    opt_a   = Some(ev)                   // bind pre half
  build inner_path (on_into_parent captures opt_foo, opt_bar)
  enter Inner
    if a matches && !handled:
      handled = true
      inner_handler(ev, &mut path, Valid{…})            // bind post half
  leave Inner
  into_parent:
    apply reshape of .inner if scheduled
    v = Validity of .inner (handled from below)
    if opt_foo: post_foo(t, &mut path, v)
    if opt_bar: post_bar(t, &mut path, v)
  if opt_a && !handled:
    handled = true
    outer_handler(ev, &mut path, v)                     // bind post half
leave Outer
```

Order: all pres (root → leaf), then leaf → root: reshape at this level, pre_post posts, then bind.

## Why `&mut` in `Valid` is sound

A post does not call `into_parent`; the framework does:

```rust
pub fn into_parent(self, handled: bool, sink: &mut Vec<Effect>) -> P {
    let v = self.read_child(handled); // apply scheduled reshape, then classify
    Extend::extend(sink, (self.on_into_parent)(parent_mut, v));
    self.parent
}
```

The `&mut N` inside `Valid` is a reborrow for the post call only. `into_parent` still owns the path and projects up after. The two never overlap.

## Path and batch

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub struct PathMut<N, P, F> {
    /* projection to N, parent P */
    on_into_parent: F, // FnOnce(&mut P, Validity<'_, N>) -> Vec<Effect>
}

fn no_post<P, N>(_path: &mut P, _: Validity<'_, N>) -> Vec<Effect> {
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

Handlers return effects; only framework code holds `&mut Vec<Effect>`. `from_fn` is crate-private — the only way to choose an `on_into_parent`.

`pre` matched ⇒ `post` ran exactly once: `opt` is `Some`, the `FnOnce` on the child path runs once inside `into_parent`. For `#[pre]` alone, that "post" is `drop`. The once-ness is ownership of the `Option` and the `FnOnce`, not a flag.

Several pre/posts on one node: one `on_into_parent` closure captures every `opt_i` and runs each post.

## Generated `Inner`

Leaf. `#[bind(a => inner_handler)]` only. No child field: `Validity` is `Valid` of the leaf node itself.

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
        // pre half: trigger → ()
        // post half: exclusive one-function body
        if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
            if a.is_matching(ev) && !*handled {
                *handled = true;
                let v = Validity::Valid(Valid {
                    node: path.get_mut(),
                    handled: true,
                });
                Extend::extend(
                    effs,
                    Into::<Vec<M::Effect>>::into(inner_handler(ev, &mut path, v)),
                );
            }
        }
        path
    }
}
```

## Generated `Outer`

pre_post posts run inside `on_into_parent`. `#[bind]` runs after `into_parent` so it can set `*handled` directly.

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
        // ----- DOWN: pres (immutable) -----
        let opt_foo = match <&FooEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Foo.is_matching(ev) => {
                Some(pre_foo(ev, ::bind::Node { parent: &path, data: () }))
            }
            _ => None,
        };
        let opt_bar = match <&BarEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if Bar.is_matching(ev) => {
                Some(pre_bar(ev, ::bind::Node { parent: &path, data: () }))
            }
            _ => None,
        };
        // bind pre half: record match + event for the post half
        let opt_a = match <&AEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if a.is_matching(ev) => Some(ev),
            _ => None,
        };

        // ----- descend: pre_post posts closed over opts -----
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, v| {
                let mut local = ::std::vec::Vec::new();
                match v {
                    Validity::Valid(mut valid) => {
                        if let Some(t) = opt_foo {
                            Extend::extend(
                                &mut local,
                                post_foo(
                                    t,
                                    parent,
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
                                    parent,
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
                            Extend::extend(
                                &mut local,
                                post_foo(t, parent, Validity::Invalidated),
                            );
                        }
                        if let Some(t) = opt_bar {
                            Extend::extend(
                                &mut local,
                                post_bar(t, parent, Validity::Invalidated),
                            );
                        }
                    }
                }
                // #[pre] alone: opt is Some(t), arm is drop(t) — no push
                local
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, handled);

        // ----- UP: reshape .inner?, pre_post posts, then bind -----
        let mut path = inner_path.into_parent(*handled, effs);

        // #[bind] post half: one function, exclusive
        if let Some(ev) = opt_a {
            if !*handled {
                *handled = true;
                let v = path.validity_of_inner(*handled); // re-derive after reshape
                Extend::extend(
                    effs,
                    Into::<Vec<M::Effect>>::into(outer_handler(ev, &mut path, v)),
                );
            }
        }
        path
    }
}
```

`#[pre]` alone: `opt = Some(pre(...))`, `on_into_parent` arm is `drop(t)`.

`#[post]` alone: `opt = Some(())` on trigger match; post gets `()`.

## Walks

### `a` only

```text
opt_foo None, opt_bar None, opt_a Some
Inner: inner_handler, handled = true
Outer into_parent: posts no-op (opts None)
Outer bind: skip (handled)
```

Batch: `inner_handler` only.

### Foo only

```text
pre_foo → opt_foo = Some(t)     // immutable
Inner: skip
Outer into_parent: post_foo(t, &mut path, Valid { handled: false })
Outer bind: skip
```

Batch: `post_foo` effects.

### Foo and `a`

```text
pre_foo → opt_foo = Some(t)
Inner: inner_handler, handled = true
Outer into_parent: post_foo(t, &mut path, Valid { handled: true })
Outer bind: skip
```

Batch: `inner_handler`, then `post_foo`.

### Reshape of `.inner`

```text
Inner: claims, schedules reshape of Outer.inner
Outer into_parent: apply reshape → Invalidated
post_foo(t, &mut path, Invalidated) if Foo matched
Outer bind: skip if handled, else runs with Invalidated
```

## Rearm

Post on the node that owns the `AndReturnHome` field:

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(path: &mut OwnerPath, node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`Valid`: rearm. `Invalidated` (left the set): skip; dropped guard cancels. No pre, no pending slot.

## Overlay

```rust
#[post(AnyKey => hide_on_change)]
fn hide_on_change(_path: &mut Path, v: Validity<'_, OverlayLayer>) -> Vec<MercuryEffect> {
    match v {
        Validity::Valid(valid) if valid.handled => vec![hide_overlay()],
        _ => vec![],
    }
}
```

## Single-owner / effects

At most one writer per field per dispatch (path consumes once). Sibling fields may each have a writer. Effects already returned as an owned `Vec` survive a shallower reshape.

## Prefactor

Shippable alone, before pre/post exist:

- `Bindings::Effect` replaces `Output`
- `dispatch` threads `effs: &mut Vec<Effect>` and `handled: &mut bool`
- today's exclusives become the `#[bind]` post half: set `handled`, push effects, path stays with the framework (`&mut`)
- `into_parent(self, _handled, _sink)` projects up; no post yet
- top-level `Some(effs)` when `handled || !effs.is_empty()`, else `None`
- `V: Into<Vec<Effect>>`; expression handlers already work (`crates/bind/tests/expr_handler.rs`)

Before/after marker:

```rust
// before
type Output;
// after
type Effect;
```

## Rules

1. pre: immutable, returns `T`, stored in `Option<T>`.
2. post: `&mut path` + `Validity` + `T`; runs iff `opt` is `Some`.
3. no pre → `T = ()`. no post → `drop(t)`.
4. `#[bind]` = one user function = pre half (trigger → `()`) + exclusive post half.
5. Several pairs → several `opt_i`, one `on_into_parent`.
6. `pre` matched ⇒ post-or-drop exactly once.
7. Reshape of a field applied in that field's `into_parent` before posts at that level.

## Tests

- pre then post on a miss: `Valid { handled: false }`
- post after leaf bind: `Valid { handled: true }`; batch order leaf-bind then parent posts
- drop counter on `T`: consumed once
- pre miss: no post
- deepest bind wins; parent bind runs when leaf misses
- reshape → post gets `Invalidated`
- `#[pre]` alone: `T` dropped, no effects from post side
- `#[post]` alone: post receives `()`
- expression position for `only_if_valid(rearm)`

## Open

- Whether `pre` may emit now-effects (`-> (T, Vec<Effect>)`) or only returns `T`. Generate is `-> T` only until a case needs down-effects.
- How a deep bind schedules a reshape of a field it does not own (carrier applied in owner's `into_parent`).
- Product nodes: one `Validity` per live child field, join in `into_parent`.
- Precise change detection (`get_mut` tracking) deferred.
