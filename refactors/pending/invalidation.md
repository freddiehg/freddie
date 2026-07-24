# Invalidation: descent schedules, ascent executes

Not done. Standalone. The generate for one tree is the design.

## The rule

1. **Descent schedules.** Matching `pre`s run (read-only), stash `T` in `Option<T>`. Matching posts and binds are recorded the same way (`opt` is `Some`). That set is final — the ascent does not re-match triggers and does not add or drop scheduled posts.
2. **Ascent executes.** Walk leaf → root. Mutation is free: binds and posts may reshape. Every scheduled post is still called, whether or not a deeper handler mutated the tree.
3. **Posts learn survival, not "who matched".** Each post receives `Validity`: either the child field is still there (`Valid(&mut N)`), or it was replaced (`Invalidated`). That is structural. It is not "something below handled the event."

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
- Immutable path only.
- Returns `T`. Framework stores `opt: Option<T>` — `Some(t)` on match, `None` otherwise.
- Match is final; ascent does not re-check the trigger.
- Does not reshape (that would change which children exist mid-schedule).

Open: whether `pre` may also push now-effects (`-> (T, Vec<Effect>)`). Generate below is `-> T` only.

### post

```rust
fn post_foo(t: T, path: &mut OuterPath, v: Validity<'_, Inner>) -> Vec<Effect>
```

- Runs on the ascent **once** iff scheduled (`opt` is `Some`), **regardless of mutation below**.
- Gets:
  - `t: T` — pre's return (moved out of the `Option`)
  - `path: &mut Path` — mutable path at this node
  - `v: Validity` — whether the guarded child field still exists after mutations so far
- Returns effects. Framework pushes them onto the batch.

```rust
enum Validity<'n, N> {
    /// Child field is still an `N`. Mutable access to it.
    Valid(&'n mut N),
    /// Child field was replaced (or is no longer an `N`). No node to hand out.
    Invalidated,
}
```

Two states. Not three. Not "handled."

### Why not a "handled" / "something matched below" bit on posts

If `Validity` (or a flag beside it) meant "any pre/post below ran," a logging pair would poison it:

```rust
#[pre_post(AnyKey => (log_pre, log_post))]  // always matches, always scheduled
```

Every key would look "handled." Rearm, overlay, exclusive gates that read that bit would fire on pure observation. Wrong.

So:

| signal | who sets it | who reads it | meaning |
|---|---|---|---|
| `opt: Option<T>` | descent, per pre/post/bind | that post only | this slot was scheduled |
| `Validity` | ascent, after reshape of this field | every post at this level | child field still `N` or not |
| `claimed: bool` | `#[bind]` only, on the way up | `#[bind]` only | an exclusive already took this event |

Posts do not see `claimed`. Binds do not put claim into `Validity`. A logging pre/post sets neither reshape nor claim.

### Defaults when one half is missing

| attribute | pre | post |
|---|---|---|
| `#[pre_post(trig => (pre, post))]` | user `pre` → `T` | user `post(t, path, v)` |
| `#[pre(trig => pre)]` | user `pre` → `T` | **drop** `t` |
| `#[post(trig => post)]` | trigger check → `()` | user `post((), path, v)` |
| `#[bind(trig => handler)]` | trigger check → `()` | exclusive body (below) |

- No pre → `T = ()`.
- No post → drop `t` at the post slot (still "ran" once as drop).
- Several pairs on one node → several independent `opt_i`, one ascent path.

### `#[bind]` is one function

One user function; logically pre + post:

- **pre half** (framework): trigger matches → `opt = Some(ev)` (or `Some(())` if the body does not need the event stashed)
- **post half** (user): runs on the way up when scheduled **and** `!claimed`; then sets `claimed = true`

```rust
fn outer_handler(ev: &AEvent, path: &mut OuterPath, v: Validity<'_, Inner>) -> Vec<Effect>

// framework:
//   if opt_a.is_some() && !claimed {
//       claimed = true;
//       effs.extend(outer_handler(ev, path, v));
//   }
```

Deepest-wins is only among binds, via `claimed`. Scheduled pre_post posts at the same level still all run; they are not exclusives.

`only_if_valid`:

```rust
fn only_if_valid<N, P>(
    f: impl FnOnce(&mut P, &mut N) -> Vec<Effect>,
) -> impl FnOnce(&mut P, Validity<'_, N>) -> Vec<Effect> {
    move |path, v| match v {
        Validity::Valid(node) => f(path, node),
        Validity::Invalidated => Vec::new(),
    }
}
```

## Walk

```text
DESCENT (schedule — final):
  enter Outer
    if Foo: opt_foo = Some(pre_foo(ev, &path))
    if Bar: opt_bar = Some(pre_bar(ev, &path))
    if a:   opt_a   = Some(ev)                 // bind scheduled
    build inner_path (posts closed over opt_foo, opt_bar)
    enter Inner
      if a: opt_a_inner scheduled (or run bind post half at leaf on the way up)
    leave Inner

ASCENT (execute — all scheduled posts run; mutation free):
    Inner bind if scheduled && !claimed → claim, may reshape
    into_parent Outer:
      apply reshape of .inner if any
      v = Valid(&mut inner) | Invalidated
      if opt_foo: post_foo(t, &mut path, v)     // always if scheduled
      if opt_bar: post_bar(t, &mut path, v)
    Outer bind if scheduled && !claimed → claim
leave Outer
```

## Why `&mut` in `Valid` is sound

```rust
pub fn into_parent(self, sink: &mut Vec<Effect>) -> P {
    let v = self.read_child(); // apply scheduled reshape, then Valid(&mut N) | Invalidated
    Extend::extend(sink, (self.on_into_parent)(parent_mut, v));
    self.parent
}
```

`&mut N` is a reborrow for the post call only. `into_parent` owns the path and projects up after. No overlap.

`claimed` is **not** an argument to `into_parent`. It is threaded on the dispatch stack for binds only.

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

Scheduled (`opt` is `Some`) ⇒ post-or-drop exactly once (`FnOnce` + ownership of `T`).

## Generated `Inner`

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
        // #[bind(a => inner_handler)] — scheduled on match, runs here (leaf ascent)
        if let Some(ev) = <&AEvent as TryFrom<_>>::try_from(event).ok() {
            if a.is_matching(ev) && !*claimed {
                *claimed = true;
                let v = Validity::Valid(path.get_mut());
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
        // ----- DESCENT: schedule (immutable pres) -----
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
        let opt_a = match <&AEvent as TryFrom<_>>::try_from(event).ok() {
            Some(ev) if a.is_matching(ev) => Some(ev),
            _ => None,
        };

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, v| {
                let mut local = ::std::vec::Vec::new();
                // every scheduled post runs; v says Valid / Invalidated only
                if let Some(t) = opt_foo {
                    Extend::extend(&mut local, post_foo(t, parent, reborrow(v)));
                }
                if let Some(t) = opt_bar {
                    Extend::extend(&mut local, post_bar(t, parent, reborrow(v)));
                }
                // #[pre] alone: opt Some(t), arm is drop(t)
                local
            },
        );

        let inner_path =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claimed);

        // ----- ASCENT: reshape, then all scheduled pre_post posts -----
        let mut path = inner_path.into_parent(effs);
        // into_parent applied reshape of .inner, built Validity, ran on_into_parent

        // ----- bind post half (exclusive; not a Validity concern) -----
        if let Some(ev) = opt_a {
            if !*claimed {
                *claimed = true;
                let v = path.validity_of_inner(); // Valid(&mut Inner) | Invalidated
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

`reborrow` / the `match v` expansion hands `Validity::Valid(&mut *node)` or `Invalidated` to each post in turn (several posts need sequential reborrows of the same `Valid` child).

## Walks

### `a` only

```text
schedule: opt_a
Inner bind: claim, run inner_handler
Outer posts: none scheduled
Outer bind: skip (claimed)
```

### Foo only

```text
schedule: opt_foo = Some(t)
Inner bind: skip
Outer into_parent: post_foo(t, path, Valid(&mut inner))
Outer bind: skip
```

### Foo and `a`

```text
schedule: opt_foo, opt_a
Inner bind: claim, inner_handler (may reshape)
Outer into_parent: post_foo(t, path, Valid | Invalidated)   // still runs
Outer bind: skip (claimed)
```

Logging pre_post on Outer would also be scheduled and would also run. It does not set `claimed`. It does not change `Validity` unless it mutates the child field.

### Reshape of `.inner`

```text
Inner bind schedules reshape of Outer.inner
Outer into_parent: apply → Invalidated
post_foo(t, path, Invalidated) if scheduled
Outer bind: skip if claimed
```

## Rearm

```rust
#[post(AnyKey => only_if_valid(rearm))]
fn rearm(path: &mut OwnerPath, node: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    node.guard = guard;
    vec![schedule]
}
```

`Valid`: rearm. `Invalidated`: skip; dropped guard cancels. No "handled" check — stay vs leave is exactly survival of the field.

## Overlay

Hide-on-activity is not a `Validity` concern. Survival of a wrapper is `Valid`/`Invalidated`. If overlay should hide when an exclusive claimed below, that is reading `claimed` in a bind-adjacent post, or (today) `set_layer` already emits hide — not a bit on every post.

```rust
#[post(AnyKey => hide_if_gone)]
fn hide_if_gone(_path: &mut Path, v: Validity<'_, OverlayLayer>) -> Vec<MercuryEffect> {
    match v {
        Validity::Invalidated => vec![hide_overlay()],
        Validity::Valid(_) => vec![],
    }
}
```

(Or keep hide only inside `set_layer`; the post form is optional.)

## Single-owner / effects

At most one writer per field per dispatch. Sibling fields may each have a writer. Effects already returned as an owned `Vec` survive a shallower reshape.

## Prefactor

- `Bindings::Effect` replaces `Output`
- `dispatch` threads `effs` and `claimed`
- exclusives become `#[bind]` post half: set `claimed`, push effects, path `&mut`
- `into_parent(self, _sink)` projects up; no post yet
- top-level `Some` when `claimed || !effs.is_empty()`
- `V: Into<Vec<Effect>>`; expression handlers already work (`expr_handler.rs`)

## Rules

1. Descent schedules; that set is final.
2. Ascent calls every scheduled post; mutation does not cancel them.
3. `Validity` = `Valid(&mut N) | Invalidated` only (structural).
4. `claimed` is bind-only; posts never see it; logging pre/posts never set it.
5. pre: immutable, `T` in `Option`. post: `&mut path` + `Validity` + `T`.
6. no pre → `()`. no post → `drop`.
7. `#[bind]` = one function = schedule on the way down + exclusive body on the way up.
8. Reshape of a field applied in that field's `into_parent` before posts at that level.

## Tests

- scheduled post runs after deep bind (including after reshape → `Invalidated`)
- logging `AnyKey` pre_post does not set `claimed`; parent bind still wins when leaf has no bind
- deepest bind wins; parent bind runs when leaf misses
- drop counter on `T`: once
- pre miss: no post
- `#[pre]` alone: drop; `#[post]` alone: `()`
- rearm: `Valid` arms, `Invalidated` skips

## Open

- Whether `pre` may emit now-effects (`-> (T, Vec<Effect>)`).
- How a deep bind schedules a reshape of a field it does not own.
- Product nodes: one `Validity` per live child field.
- Whether any post needs a third state (e.g. "child still `N` but a descendant field reshaped") — not required for rearm; defer until a case needs it.
