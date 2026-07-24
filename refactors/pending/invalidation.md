# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root; mutation is free; each post is told whether its child field still exists. Exclusive deepest-wins is a separate `claimed` bit used only by `#[bind]`.

## Paths: shared down, owned up

The path type is the normal owned `PathMut` (same as today). What changes is when the handler may hold it.

- **Descent (pre):** the framework still owns the path for the walk. Pre gets a shared borrow: `Node<&P, D>`. Read-only; no reshape.
- **Ascent (post / bind):** `into_parent` has recovered this level's path as an owned `P` again (built the usual way: `from_fn` on the way down, `into_parent` on the way up). Post and bind receive that owned path: `Node<P, D>` — same shape as today's exclusive handlers. `get_mut`, `ascend_mut`, the usual tools.

There is no parallel "`&mut Path`" API for posts. Mutation is allowed because ownership of the normal path is back at this level on the way up, not because the signature is a mutable reference.

To thread ownership through several posts at one level, each post/bind returns the path with its effects:

```rust
fn post_foo(
    t: TFoo,
    node: Node<OuterPath, ()>,
    v: Validity,
) -> (Vec<M::Effect>, OuterPath)
```

(A later sugar can hide the path in the return for bodies that only `get_mut`.)

## Developer experience

```rust
#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(Foo => (pre_foo, post_foo), Bar => (pre_bar, post_bar))]
#[pre(Baz => track)]                 // post is drop
#[post(Qux => guard)]                // pre is trigger → ()
#[bind(KeyA => outer_handler)]       // one function; exclusive
struct Outer {
    #[resolve_into]
    inner: Inner,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner;
```

```rust
// pre: shared path
fn pre_foo(ev: &FooEvent, node: Node<&OuterPath, ()>) -> TFoo { ... }

// post: owned path in, path out, Validity tag for the resolve_into child
fn post_foo(
    t: TFoo,
    node: Node<OuterPath, ()>,
    v: Validity,
) -> (Vec<M::Effect>, OuterPath) {
    match v {
        Validity::Valid => {
            // node.parent.get_mut().inner is still Inner
            (vec![], node.parent)
        }
        Validity::Invalidated => (vec![], node.parent),
    }
}

// bind: one function; owned path; exclusive via claimed
fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    v: Validity,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    v: Validity,
) -> (Vec<M::Effect>, InnerPath) { ... }

// sugar: act only when the child survived
fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}
// #[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
```

Expression positions work as today (`#handler(…)` splice). Pinned by `crates/bind/tests/expr_handler.rs`.

## Semantics

### Schedule on the way down (final)

For each pre/post/bind on the node, if the trigger matches:

- `#[pre_post]` / `#[pre]`: call pre with `Node<&P, D>`, store `opt_i = Some(t)`
- `#[post]`: store `opt_i = Some(())`
- `#[bind]`: store `opt_i = Some(ev)`

If the trigger misses, `opt_i = None`. Ascent never re-checks triggers and never changes which opts are `Some`.

```rust
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T
```

No reshape on the descent. Whether pre may also return now-effects is open; generate is `-> T` only.

### Execute on the way up (all scheduled posts run)

Leaf to root. At each level the framework holds an owned path again.

1. Apply any reshape scheduled for this level's child field.
2. Build `Validity` of that field.
3. For each `opt_i` that is `Some`, run the post with the owned path (or `drop(t)` if `#[pre]` alone); take path back from the return.
4. For each scheduled `#[bind]`, if `!claimed`, set `claimed = true`, run with owned path, take path back.

A scheduled post always runs. `Invalidated` is how it learns it lost the child.

```rust
enum Validity {
    /// Child field is still the scheduled type. Projections through the path are sound.
    Valid,
    /// Child field was replaced.
    Invalidated,
}
```

### Three signals (do not conflate)

- `opt: Option<T>` — this slot was scheduled (descent). Read only by that post.
- `Validity` — child field still that type or not (structural). Every post at this level sees it.
- `claimed: bool` — a `#[bind]` already took this event. Bind-only. Posts never see it.

A logging `#[pre_post(AnyKey => …)]` schedules and runs; it does not set `claimed` and does not change `Validity` unless it mutates the child.

### Defaults

- `#[pre_post(trig => (pre, post))]` — user pre → `T`, user post
- `#[pre(trig => pre)]` — user pre → `T`, post is `drop(t)`
- `#[post(trig => post)]` — pre is trigger → `()`, user post gets `()`
- `#[bind(trig => handler)]` — pre is trigger → stash event, post is exclusive body

Several pairs on one node: several independent `opt_i`, one `on_into_parent` closure.

### `#[bind]` is one function

```rust
// user:
fn handler(
    ev: &E,
    node: Node<P, ()>,
    v: Validity,
) -> (impl Into<Vec<Effect>>, P);

// framework (path owned at this level; posts already ran):
if let Some(ev) = opt {
    if !*claimed {
        *claimed = true;
        let (out, path) = handler(ev, Node { parent: path, data: () }, v);
        Extend::extend(effs, out.into());
        // path continues to the caller
    }
}
```

Deepest-wins is only among binds. pre_post posts at the same level all still run (before the bind).

### `only_if_valid`

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, Validity) -> (Vec<Effect>, P) {
    move |mut node, v| {
        let effs = match v {
            Validity::Valid => f(project(&mut node.parent)),
            Validity::Invalidated => Vec::new(),
        };
        (effs, node.parent)
    }
}
```

## Types

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

#[derive(Clone, Copy)]
pub enum Validity {
    Valid,
    Invalidated,
}

pub struct PathMut<N, P, F> {
    // owned parent P + projections to N (as today)
    on_into_parent: F, // FnOnce(P, Validity) -> (P, Vec<Effect>)
}

fn no_post<P>(parent: P, _: Validity) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(P, Validity) -> (P, Vec<Effect>),
{
    /// Apply reshape of the child field if any, classify, run posts, return parent.
    pub fn into_parent(self, sink: &mut Vec<Effect>) -> P {
        let v = self.classify_after_reshape(); // Validity
        let (parent, post_effs) = (self.on_into_parent)(self.parent, v);
        Extend::extend(sink, post_effs);
        parent
    }
}

pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
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
    let mut claimed = false;
    let _path = <N as Dispatch<M>>::dispatch(path, event, &mut effs, &mut claimed);
    if claimed || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

Handlers return effects; only framework code holds the batch sink. `from_fn` is crate-private. `claimed` is not an argument to `into_parent`.

## Generated code

### Inner (leaf, one bind)

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) && !*claimed {
                *claimed = true;
                let (out, path) = inner_handler(
                    ev,
                    ::bind::Node {
                        parent: path,
                        data: (),
                    },
                    ::bind::Validity::Valid,
                );
                ::core::iter::Extend::extend(
                    effs,
                    ::core::convert::Into::<
                        ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
                    >::into(out),
                );
                return path;
            }
        }
        path
    }
}
```

### Outer (two pre_posts + one bind)

```rust
impl Dispatch<M> for Outer {
    fn dispatch<'a>(
        mut path: <Outer as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Outer as Place>::Path<'a>
    where
        Self: 'a,
    {
        // ----- descent: schedule (shared borrow) -----
        let opt_foo = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = Foo;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(pre_foo(
                    ev,
                    ::bind::Node {
                        parent: &path,
                        data: (),
                    },
                ))
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        let opt_bar = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = Bar;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(pre_bar(
                    ev,
                    ::bind::Node {
                        parent: &path,
                        data: (),
                    },
                ))
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        let opt_a = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(ev)
            } else {
                ::core::option::Option::None
            }
        } else {
            ::core::option::Option::None
        };

        // move owned path into child; on_into_parent re-owns parent for posts
        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, v| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(t) = opt_foo {
                    let (out, p) = post_foo(
                        t,
                        ::bind::Node {
                            parent: path,
                            data: (),
                        },
                        v,
                    );
                    ::core::iter::Extend::extend(&mut local, out);
                    path = p;
                }
                if let ::core::option::Option::Some(t) = opt_bar {
                    let (out, p) = post_bar(
                        t,
                        ::bind::Node {
                            parent: path,
                            data: (),
                        },
                        v,
                    );
                    ::core::iter::Extend::extend(&mut local, out);
                    path = p;
                }
                (path, local)
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claimed);

        // ----- ascent: reshape + pre_post posts -----
        let mut path = inner_path.into_parent(effs);

        // ----- bind last at this level -----
        if let ::core::option::Option::Some(ev) = opt_a {
            if !*claimed {
                *claimed = true;
                let v = path.validity_of_inner();
                let (out, p) = outer_handler(
                    ev,
                    ::bind::Node {
                        parent: path,
                        data: (),
                    },
                    v,
                );
                ::core::iter::Extend::extend(
                    effs,
                    ::core::convert::Into::<
                        ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
                    >::into(out),
                );
                path = p;
            }
        }
        path
    }
}
```

### `#[pre]` / `#[post]` alone in the closure

```rust
// #[pre(Baz => track)]
if let ::core::option::Option::Some(t) = opt_baz {
    ::core::mem::drop(t);
}

// #[post(Qux => guard)]
if let ::core::option::Option::Some(()) = opt_qux {
    let (out, p) = guard(
        (),
        ::bind::Node {
            parent: path,
            data: (),
        },
        v,
    );
    ::core::iter::Extend::extend(&mut local, out);
    path = p;
}
```

## Walk

```text
DESCENT
  Outer owns path
  pre_foo? pre_bar? with &path → opt_*
  opt_a? (bind scheduled)
  move path into inner_path (from_fn)
  Inner: bind KeyA?

ASCENT
  Inner: if scheduled && !claimed → claimed, inner_handler(owned path) → path back
  Outer into_parent:
    reshape .inner, v = Valid | Invalidated
    post_foo? post_bar? each with owned path in/out
  Outer bind: if scheduled && !claimed → outer_handler(owned path) → path back
```

### `KeyA` only

```text
Inner bind runs, claimed = true, path returned for into_parent
Outer posts: none
Outer bind skips
```

### `Foo` only

```text
opt_foo = Some(t)
Inner bind skips
Outer post_foo(t, owned path, Valid) → path back
Outer bind skips
```

### `Foo` and `KeyA`

```text
Inner bind may reshape .inner, claimed = true, path returned
Outer post_foo still runs (Valid or Invalidated)
Outer bind skips
```

### Logging `AnyKey` pre_post also present

```text
also schedules, also runs
does not set claimed
does not change Validity unless it mutates the child
```

## Rearm

```rust
#[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`Valid`: rearm. `Invalidated`: skip; `Drop` of the guard cancels the timer.

## Prefactor

Shippable alone, before pre/post attributes exist.

Before: `type Output`, `ControlFlow<Output, Path>`, exclusive takes path by value and `Break`s.

After:

- `type Effect`
- `dispatch` threads `effs` and `claimed`, always returns `Path`
- exclusive is the bind post half: set `claimed`, push effects, return path
- `into_parent(self, sink)` exists; no post yet
- top-level `Some` when `claimed || !effs.is_empty()`
- `V: Into<Vec<Effect>>`; expression handlers already work

Mercury binds that today `ascend_mut` + `set_layer` wait on the reshape carrier (Open): under full ascent they should schedule the layer replace for the owner's `into_parent`, then return the path, rather than consuming the path mid-walk. Prefactor may keep today's by-value exclusive + `Break` for a behavior-identical cut; the feature wants full ascent + path returned.

## Rules

1. Descent schedules; that set is final.
2. Ascent runs every scheduled post; mutation does not cancel them.
3. Pre: shared path (`Node<&P, D>`). Post/bind: owned path (`Node<P, D>`), path returned with effects.
4. `Validity = Valid | Invalidated` only (tag).
5. `claimed` is bind-only; logging never sets it.
6. no pre → `()`. no post → `drop`.
7. `#[bind]` = one function = schedule down + exclusive body up (after pre_post posts at that level).
8. Reshape of a field applied in that field's `into_parent` before posts at that level.

## Tests

- scheduled post runs after deep bind, including `Invalidated`
- logging `AnyKey` pre_post does not set `claimed`
- deepest bind wins; parent bind runs when leaf misses
- path is returned through two posts at one level
- drop counter on `T`: once
- pre miss: no post
- `#[pre]` alone: drop; `#[post]` alone: `()`
- expression post: `only_if_valid(project, rearm)`

## Open

- Whether `pre` may emit now-effects (`-> (T, Vec<Effect>)`).
- How a deep bind schedules a reshape of a field it does not own (carrier applied in owner's `into_parent`). Until then, binds that `ascend_mut` + `set_layer` and path return do not compose cleanly.
- Sugar so user posts can write `-> Vec<Effect>` while the derive still threads path.
- Product nodes: one `Validity` per live child field.
- Fallbacks that must not run if something claimed the event (e.g. root `AnyKey` passthrough). Closer to `claimed` than to `Validity`. Deferred.
- Third Validity state: deferred.
