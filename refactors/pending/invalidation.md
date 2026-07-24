# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds will run. That set is final. Ascent runs every scheduled post leaf to root; mutation is free; each post is told whether its child field still exists. Exclusive deepest-wins is a separate `claimed` bit used only by `#[bind]`.

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

What the user writes:

```rust
// pre: immutable path, returns carriage value T
fn pre_foo(ev: &FooEvent, node: Node<&OuterPath, ()>) -> TFoo { ... }

// post: mut path, carriage T, Validity of the resolve_into child (tag only — see below)
fn post_foo(t: TFoo, path: &mut OuterPath, v: Validity) -> Vec<M::Effect> {
    match v {
        Validity::Valid => {
            // path.get_mut().inner is still Inner (invariant of Valid)
            vec![]
        }
        Validity::Invalidated => vec![],
    }
}

// bind: one function; runs on ascent if scheduled and nothing deeper claimed
fn outer_handler(ev: &KeyEvent, path: &mut OuterPath, v: Validity) -> Vec<M::Effect> { ... }

fn inner_handler(ev: &KeyEvent, path: &mut InnerPath, v: Validity) -> Vec<M::Effect> { ... }

// sugar: only act when the child survived; body gets &mut Child, not the parent path
fn rearm(node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
// #[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
```

Expression positions work as today (`#handler(…)` splice): `only_if_valid(project, rearm)` is an expression that yields the post. Pinned by `crates/bind/tests/expr_handler.rs`.

## Semantics

### Schedule on the way down (final)

For each pre/post/bind on the node, if the trigger matches:

- `#[pre_post]` / `#[pre]`: call pre with immutable path, store `opt_i = Some(t)`
- `#[post]`: store `opt_i = Some(())`
- `#[bind]`: store `opt_i = Some(ev)` (event stashed for the body)

If the trigger misses, `opt_i = None`. The ascent never re-checks triggers and never changes which opts are `Some`.

Pre signature:

```rust
fn pre(ev: &SourceEvent, node: Node<&Path, D>) -> T
```

Immutable only. No reshape on the descent (that would change which children exist mid-schedule).

Whether pre may also return now-effects (`(T, Vec<Effect>)`) is open; generate is `-> T` only.

### Execute on the way up (all scheduled posts run)

Leaf to root. At each level:

1. Apply any reshape scheduled for this level's child field.
2. Build `Validity` of that field.
3. For each `opt_i` that is `Some`, run the post (or `drop(t)` if `#[pre]` alone).
4. For each scheduled `#[bind]`, if `!claimed`, set `claimed = true` and run the handler.

Mutation during (3) or (4) at deeper levels is already visible as `Validity` here. Mutation here is visible to shallower levels. A scheduled post always runs; `Invalidated` is how it learns it lost the child.

```rust
enum Validity {
    Valid,
    Invalidated,
}
```

### Three signals (do not conflate)

- `opt: Option<T>` — this slot was scheduled. Set on descent. Read only by that post.
- `Validity` — child field still `N` or not. Structural. Every post at this level sees it.
- `claimed: bool` — a `#[bind]` already took this event. Bind-only. Posts never see it.

A logging `#[pre_post(AnyKey => (log_pre, log_post))]` schedules and runs, and does not set `claimed` and does not change `Validity` unless it mutates the child. Exclusive deepest-wins and rearm stay correct.

### Defaults

- `#[pre_post(trig => (pre, post))]` — user pre → `T`, user post
- `#[pre(trig => pre)]` — user pre → `T`, post is `drop(t)`
- `#[post(trig => post)]` — pre is trigger → `()`, user post gets `()`
- `#[bind(trig => handler)]` — pre is trigger → stash event, post is exclusive body

Several pairs on one node: several independent `opt_i`, one `on_into_parent` closure.

### `#[bind]` is one function

```rust
// user:
fn handler(ev: &E, path: &mut Path, v: Validity) -> impl Into<Vec<Effect>>;

// framework post half:
if let Some(ev) = opt {
    if !*claimed {
        *claimed = true;
        Extend::extend(effs, handler(ev, path, v).into());
    }
}
```

Deepest-wins is only among binds. pre_post posts at the same level all still run.

### `only_if_valid`

Projects the child for the body when `Valid`, so the body never holds `&mut Path` and `&mut Child` at once:

```rust
fn only_if_valid<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(&mut P, Validity) -> Vec<Effect> {
    move |path, v| match v {
        Validity::Valid => f(project(path)),
        Validity::Invalidated => Vec::new(),
    }
}

// use:
// #[post(AnyKey => only_if_valid(|p| &mut p.get_mut().return_home, rearm))]
```

## Types

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

pub enum Validity {
    Valid,
    Invalidated,
}

pub struct PathMut<N, P, F> {
    // projection to N, parent P
    on_into_parent: F, // FnOnce(&mut P, Validity) -> Vec<Effect>
}

fn no_post<P>(_path: &mut P, _: Validity) -> Vec<Effect> {
    Vec::new()
}

impl<N, P, F> PathMut<N, P, F>
where
    F: FnOnce(&mut P, Validity) -> Vec<Effect>,
{
    /// Apply reshape of the child field if any, classify, run posts, return parent.
    pub fn into_parent(self, sink: &mut Vec<Effect>) -> P {
        let mut parent = self.parent;
        let v = self.apply_reshape_and_classify(&mut parent); // Validity
        Extend::extend(sink, (self.on_into_parent)(&mut parent, v));
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

Handlers return effects; only framework code holds `&mut Vec<Effect>`. `from_fn` is crate-private.

`claimed` is not an argument to `into_parent`. It is threaded on the dispatch stack for binds only.

## Generated code

### Inner (leaf, one bind)

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        mut path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claimed: &mut bool,
    ) -> <Inner as Place>::Path<'a>
    where
        Self: 'a,
    {
        // schedule + execute bind (leaf has no child to into_parent through)
        if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) && !*claimed {
                *claimed = true;
                ::core::iter::Extend::extend(
                    effs,
                    ::core::convert::Into::<::std::vec::Vec<<M as ::bind::Bindings>::Effect>>::into(
                        inner_handler(ev, &mut path, ::bind::Validity::Valid),
                    ),
                );
            }
        }
        path
    }
}
```

### Outer (two pre_posts + one bind + resolve_into child)

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
        // ----- descent: schedule -----
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

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, v| {
                let mut local = ::std::vec::Vec::new();
                if let ::core::option::Option::Some(t) = opt_foo {
                    ::core::iter::Extend::extend(&mut local, post_foo(t, parent, v));
                }
                if let ::core::option::Option::Some(t) = opt_bar {
                    ::core::iter::Extend::extend(&mut local, post_bar(t, parent, v));
                }
                // #[pre] alone would drop(t) here instead of calling a post
                local
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claimed);

        // ----- ascent: reshape + pre_post posts -----
        let mut path = inner_path.into_parent(effs);

        // ----- bind post half -----
        if let ::core::option::Option::Some(ev) = opt_a {
            if !*claimed {
                *claimed = true;
                let v = path.validity_of_inner(); // Valid | Invalidated after reshape
                ::core::iter::Extend::extend(
                    effs,
                    ::core::convert::Into::<
                        ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
                    >::into(outer_handler(ev, &mut path, v)),
                );
            }
        }
        path
    }
}
```

`Validity` is `Copy`, so several posts in one closure all receive the same tag. Each post that needs the child projects through `path` under the `Valid` invariant.

### `#[pre]` alone / `#[post]` alone in the closure

```rust
// #[pre(Baz => track)] — opt_baz: Option<TBaz>, post is drop
if let ::core::option::Option::Some(t) = opt_baz {
    ::core::mem::drop(t);
}

// #[post(Qux => guard)] — opt_qux: Option<()>
if let ::core::option::Option::Some(()) = opt_qux {
    ::core::iter::Extend::extend(&mut local, guard((), parent, v));
}
```

## Walk

```text
DESCENT (schedule — final)
  Outer: opt_foo? opt_bar? opt_a?
  build inner_path (closure captures opts)
  Inner: bind KeyA?

ASCENT (execute — every scheduled post runs)
  Inner: if opt_a && !claimed → claimed=true, inner_handler(…, Valid)
  Outer into_parent:
    apply reshape of .inner
    v = Valid | Invalidated
    if opt_foo → post_foo(t, path, v)
    if opt_bar → post_bar(t, path, v)
  Outer: if opt_a && !claimed → claimed=true, outer_handler(…, v)
```

### `KeyA` only

```text
schedule opt_a at Outer and Inner
Inner bind runs, claimed = true
Outer posts: none
Outer bind skips
```

### `Foo` only

```text
schedule opt_foo = Some(t)
Inner bind skips
Outer post_foo(t, path, Valid)
Outer bind skips
```

### `Foo` and `KeyA`

```text
schedule opt_foo, opt_a
Inner bind runs (may reshape .inner), claimed = true
Outer post_foo still runs — Valid or Invalidated
Outer bind skips
```

### Logging `AnyKey` pre_post also present

```text
also schedules, also runs on ascent
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

Stay in the set: `Valid`, rearm. Leave: `Invalidated`, skip; `Drop` of the guard cancels the timer.

## Prefactor

Shippable alone, before pre/post attributes exist. Behavior-identical exclusive dispatch once exclusives borrow the path and set `claimed`.

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

- mercury: `type Effect = MercuryEffect`
- exclusive handlers take `&mut Path`, set `claimed`, push `V: Into<Vec<Effect>>`
- `into_parent(self, _sink)` exists; body still just returns parent
- `from_fn` crate-private
- `impl From<MercuryEffect> for Vec<MercuryEffect>`

Handler before/after (shape only; reshape carrier still open):

```rust
// before
fn to_nav<'a, E, P: AscendMut<MercuryPath<'a>>>(
    _ev: &E,
    node: Node<P, ()>,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = node.parent.ascend_mut().set_layer(nav);
    effects.push(timer);
    effects
}

// after — path borrowed so the framework keeps the path for ascent
fn to_nav(
    _ev: &KeyEvent,
    path: &mut LayerPath<'_>,
    _v: Validity,
) -> Vec<MercuryEffect> {
    let (nav, timer) = NavLayer::new();
    let mut effects = path.ascend_mut().set_layer(nav);
    effects.push(timer);
    effects
}
```

Exact path types for mercury binds wait on the reshape carrier (Open). Prefactor: borrowed path + `claimed`, no `ControlFlow`.

## Rules

1. Descent schedules; that set is final.
2. Ascent runs every scheduled post; mutation does not cancel them.
3. `Validity = Valid | Invalidated` only (tag; child access is a projection of `path` when `Valid`).
4. `claimed` is bind-only; posts never see it; logging never sets it.
5. pre: immutable, `T` in `Option`. post: `&mut path` + `Validity` + `T`.
6. no pre → `()`. no post → `drop`.
7. `#[bind]` = one function = schedule down + exclusive body up.
8. Reshape of a field applied in that field's `into_parent` before posts at that level.
9. One live `&mut` at a time: no `Valid(&mut N)` beside `&mut Path`.

## Tests

`crates/bind/tests/`, tree shaped like Outer/Inner:

- scheduled post runs after deep bind, including when reshape yields `Invalidated`
- logging `AnyKey` pre_post does not set `claimed`; parent bind still wins when leaf has no bind
- deepest bind wins; parent bind runs when leaf misses
- drop counter on `T`: consumed once
- pre miss: no post
- `#[pre]` alone: drop; `#[post]` alone: receives `()`
- two posts both receive the same `Validity` tag; each may project the child when `Valid`
- expression post: `only_if_valid(project, rearm)`

Mercury: rearm moves off `handle`'s discriminant check onto `#[post(AnyKey => only_if_valid(project, rearm))]`; transition tests keep the same effects.

## Open

- Whether `pre` may emit now-effects (`-> (T, Vec<Effect>)`).
- How a deep bind schedules a reshape of a field it does not own (carrier applied in owner's `into_parent`).
- Product nodes: one `Validity` per live child field, join in `into_parent`.
- Fallbacks that must not run if something "handled" the event (e.g. root `AnyKey` passthrough only when no special key bind claimed). That is not `Validity` and not "any pre/post scheduled." It is closer to `claimed`, or a separate policy bit. Deferred; do not overload `Validity` or logging-safe signals to solve it.
- Third Validity state (survived but descendant reshaped): deferred until a case needs it.
