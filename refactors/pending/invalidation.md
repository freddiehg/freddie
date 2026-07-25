# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post. Where the owned path sits is not a counter. It is the value dispatch returns.

## Why not hop counts

`invalidation_depth` and `ascent_hops` are two sources of truth for one fact: which path segment is still owned. They can disagree. `mutation()` is a comparison of those two numbers while a `PathMut` to a killed segment may still be sitting in an `Ascent` field. The borrow checker cannot see the counters; it only sees the path value. That model is wrong.

One fact, one value: the path you still hold. Encode jumps in the return type.

## Model

Spine `0 → A → B → C`. Descent builds a path to `C`:

```text
PathMut<C, PathMut<B, PathMut<A, 0Path>>>
```

`C`'s dispatch returns a russian doll of `Result`s — expected parent path, or already further up:

```text
Result<BPath, Result<APath, 0Path>>
```

```text
Ok(b)           // still own B (normal peel from C)
Err(Ok(a))      // B is gone; own A only
Err(Err(z))     // A and B gone; own 0 only
```

Same idea one level up:

```text
B returns  Result<APath, 0Path>
A returns  0Path                 // or Result<0Path, !> if uniformity wants it
```

Each non-root node’s ascent type is:

```rust
// N's path is PathMut<N, ParentPath>. Parent place is P.
// N peels to ParentPath on the Ok side; Err is whatever P returns to its parent.
type Ascent<'a> = Result<ParentPath<'a>, <P as Dispatch<M>>::Ascent<'a>>;
```

Root does not return a doll; free `dispatch` owns the process and ends at `0Path` / effects.

### Parent matches the doll

```rust
// B::dispatch — child is C
let child_path = PathMut::from_fn(b_path, ...);
match <C as Dispatch<M>>::dispatch(child_path, event, effs, claim) {
    Ok(mut b_path) => {
        // Own B. C layer already peeled inside C or by Ok construction.
        // B posts run here with &mut b_path (Intact).
        // exclusive via claim on &mut b_path / a thin wrapper if needed.
        Ok(b_path.into_parent()) // Result::Ok for B's Ascent = Result<APath, 0Path>
    }
    Err(rest) => {
        // rest: Result<APath, 0Path> — B's own Ascent type.
        // No B path. B was skipped by a jump. B posts still run (scheduled)
        // but only with pre-snapped descent data + knowledge they are dropped.
        // Then propagate: already the type B must return.
        // (run MaybeDropped posts first)
        rest
    }
}
```

Normal walk C → B → A → 0 is a chain of `Ok` peels. A kill that jumps past B is `Err` from C carrying `Ok(a)` or `Err(z)`; B never receives a fabricated B path.

### Kill

No `invalidate(d)` counter. Kill peels with `into_parent` and returns the matching `Err` nest.

```rust
// From C, jump past B, stop at A:
let b = c_path.into_parent();
let a = b.into_parent();
// C::Ascent = Result<BPath, Result<APath, 0Path>>
Err(Ok(a))

// From C, jump to 0:
let b = c_path.into_parent();
let a = b.into_parent();
let z = a.into_parent();
Err(Err(z))
```

Helpers can wrap that so user code does not write the nests by hand; the type still is the nest.

### Claim

Separate carrier, root-owned slot, reborrowed for exclusive. Not part of the path doll.

```rust
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}
```

### Posts

Scheduled set is final; posts run on both arms.

- `Ok(path)` at this level: posts get `&mut path` (the path still owned here).
- `Err(_)` at this level: no path to this node. Posts use pre-snapped descent data only (`#[pre_post]`). Path mutation posts (`only_if_intact`) are the Ok arm only.

There is no `mutation()` bit beside a path that might lie. Intact means Ok. MaybeDropped means Err.

## Types (`crates/bind`)

```rust
pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

impl<'c> Claim<'c> {
    pub fn new(slot: &'c mut Option<()>) -> Self {
        Self { slot }
    }

    pub fn is_taken(&self) -> bool {
        self.slot.is_some()
    }

    pub fn try_take(&mut self) -> Option<()> {
        if self.slot.is_some() {
            None
        } else {
            *self.slot = Some(());
            Some(())
        }
    }

    pub fn with_exclusive<P, E>(
        &mut self,
        path: &mut P,
        body: impl FnOnce(&mut P) -> Vec<E>,
    ) -> Vec<E> {
        match self.try_take() {
            None => Vec::new(),
            Some(()) => body(path),
        }
    }
}

/// Build C::Ascent = Result<B, Result<A, Z>> by peeling.
/// Framework/helper territory; exact API open but must produce the doll, not counters.
pub fn peel_ok<N, P>(path: laserbeam::PathMut<N, P>) -> Result<P, std::convert::Infallible> {
    Ok(path.into_parent())
}
```

Dispatch:

```rust
pub trait Dispatch<M: Bindings>: Place {
    /// Nested Result doll: Ok = parent path still owned; Err = parent's Ascent (already further up).
    type Ascent<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a;
}
```

Associated type for a place whose path is `PathMut<Node, ParentPath>` and whose parent place is `Parent`:

```rust
// emitted by derive
type Ascent<'a> = Result<
    <Self::Path<'a> as HasParent>::Parent, // ParentPath
    <Parent as Dispatch<M>>::Ascent<'a>,
>;
```

Root place (`Path = &mut Root`): `type Ascent<'a> = ();` after running root posts, or the free function never uses root’s Ascent and only drains `effs`.

## User signatures

```rust
// pre — descent; shared path
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

// post on Ok arm — this level still owns P
fn post(pre_return: T, path: &mut P) -> Vec<M::Effect>;
fn post(path: &mut P) -> Vec<M::Effect>;

// post on Err arm — no P; pre-snap only (or unit)
fn post_dropped(pre_return: T) -> Vec<M::Effect>;
// or one post that is only registered for Ok via only_if_intact

// exclusive — only when this level owns path (Ok path into exclusive)
fn exclusive(ev: &SourceEvent, path: &mut P) -> Vec<M::Effect>;
// kill returns peels by ending dispatch with Err(...) — see leaf expand
```

Exact sugar for “handler peels and returns Err nest” is part of F2; the return type of `dispatch` is the doll either way.

## PathMut (laserbeam)

Unchanged. `into_parent` is the only peel. `get` re-derives; after a jump you do not hold the intermediate path type, so you cannot call `get` on it.

## Level order

```text
DESCENT: schedule opts (pre may snap)

if child:
  child_path = PathMut::from_fn(path, ...)
  match Child::dispatch(child_path, event, effs, claim) {
    Ok(path) => {
      // posts Intact with &mut path
      // exclusive with claim + &mut path
      Ok(path.into_parent())            // if this node is not root
    }
    Err(rest) => {
      // posts MaybeDropped without this level's path
      rest                              // already Self::Ascent's Err payload shape
    }
  }

if leaf:
  // exclusive may peel into Err nest
  // else Ok(path.into_parent())
```

## DX example

`0` = root path, `A` unused in the small tree, `B` = Outer, `C` = Inner.

```rust
// Inner (C) Ascent = Result<OuterPath, RootPath>   // two levels under root for the demo
// Outer (B) Ascent = RootPath                      // or Result<RootPath, !>
```

Full spine `0-A-B-C` in the abstract model; demo can be `Root-Outer-Inner` with doll depth 2:

```rust
// Inner::Ascent<'a> = Result<OuterPath<'a>, RootPath<'a>>
// Outer::Ascent<'a> = RootPath<'a>
```

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChildId(u64);

struct TimerGuard {
    id: TimerId,
}

struct TimerId(u64);

impl TimerId {
    fn fresh() -> Self {
        Self(1)
    }
}

impl TimerGuard {
    fn armed(id: TimerId) -> Self {
        Self { id }
    }
}

struct AndReturnHome {
    guard: TimerGuard,
}

enum DemoEffect {
    LogDestroyed(ChildId),
    ScheduleTimer(TimerId),
    SetLayerHome,
}

fn log_destroyed(id: ChildId) -> DemoEffect {
    DemoEffect::LogDestroyed(id)
}

fn arm_return_home() -> (TimerGuard, DemoEffect) {
    let id = TimerId::fresh();
    (TimerGuard::armed(id), DemoEffect::ScheduleTimer(id))
}

#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(AnyKey => (snap_child_id, after_child))]
#[post(AnyKey => only_if_intact(rearm))]
#[bind(KeyA => outer_handler)]
struct Outer {
    #[resolve_into]
    inner: Inner,
    return_home: AndReturnHome,
}

#[derive(Bind)]
#[node(parent = OuterPath)]
#[binds(M)]
#[bind(KeyA => inner_handler)]
struct Inner {
    id: ChildId,
}

fn snap_child_id(_ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

// Ok arm only — has OuterPath
fn after_child_ok(id: ChildId, path: &mut OuterPath) -> Vec<DemoEffect> {
    let live = path.get().inner.id;
    debug_assert_eq!(live, id);
    vec![]
}

// Err arm — no OuterPath; snapped id only
fn after_child_dropped(id: ChildId) -> Vec<DemoEffect> {
    vec![log_destroyed(id)]
}

fn rearm(path: &mut OuterPath) -> Vec<DemoEffect> {
    let (guard, schedule) = arm_return_home();
    path.get_mut().return_home.guard = guard;
    vec![schedule]
}

fn outer_handler(_ev: &KeyEvent, _path: &mut OuterPath) -> Vec<DemoEffect> {
    vec![DemoEffect::SetLayerHome]
}

// Kill: peel Outer away, return Err(root). Type is Inner::Ascent.
// Sugar TBD; shown as the leaf dispatch result.
fn inner_handler(_ev: &KeyEvent, path: InnerPath) -> (Vec<DemoEffect>, Result<OuterPath, RootPath>) {
    let outer = path.into_parent();
    let root = outer.into_parent();
    (vec![], Err(root))
}
```

## Generated: Inner (leaf)

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Inner {
    type Ascent<'a> = ::core::result::Result<
        <Outer as ::bind::Place>::Path<'a>,
        <Outer as ::bind::Dispatch<M>>::Ascent<'a>,
    >
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: <Inner as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0: ::core::option::Option<&KeyEvent> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
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

        if let ::core::option::Option::Some(ev) = opt_0 {
            match claim.try_take() {
                ::core::option::Option::None => {}
                ::core::option::Option::Some(()) => {
                    // inner_handler consumes path and returns the doll + effects
                    let (e, ascent) = inner_handler(ev, path);
                    ::core::iter::Extend::extend(effs, e);
                    return ascent;
                }
            }
        }

        // no exclusive kill: normal peel to Outer
        ::core::result::Result::Ok(::laserbeam::PathMut::into_parent(path))
    }
}
```

## Generated: Outer

```rust
#[automatically_derived]
impl ::bind::Dispatch<M> for Outer
where
    Inner: ::bind::Dispatch<M>,
{
    type Ascent<'a> = <Root as ::bind::Place>::Path<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        mut path: <Outer as ::bind::Place>::Path<'a>,
        event: &<M as ::bind::Bindings>::Event,
        effs: &mut ::std::vec::Vec<<M as ::bind::Bindings>::Effect>,
        claim: &mut ::bind::Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a,
    {
        let opt_0: ::core::option::Option<ChildId> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = AnyKey;
                if ::bind::EventTrigger::is_matching(&trigger, ev) {
                    ::core::option::Option::Some(snap_child_id(
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

        let opt_1: bool =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
            {
                let trigger = AnyKey;
                ::bind::EventTrigger::is_matching(&trigger, ev)
            } else {
                false
            };

        let opt_2: ::core::option::Option<&KeyEvent> =
            if let ::core::option::Option::Some(ev) =
                ::core::result::Result::ok(::core::convert::TryFrom::try_from(event))
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
            |p: &mut <Outer as ::bind::Place>::Path<'a>| &mut p.get_mut().inner,
            |p: &<Outer as ::bind::Place>::Path<'a>| &p.get().inner,
        );

        match <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs, claim) {
            ::core::result::Result::Ok(mut path) => {
                // Own OuterPath. Inner peeled.
                if let ::core::option::Option::Some(id) = opt_0 {
                    ::core::iter::Extend::extend(effs, after_child_ok(id, &mut path));
                }
                if opt_1 {
                    ::core::iter::Extend::extend(effs, rearm(&mut path));
                }
                if let ::core::option::Option::Some(ev) = opt_2 {
                    let e = claim.with_exclusive(&mut path, |path| outer_handler(ev, path));
                    ::core::iter::Extend::extend(effs, e);
                }
                path.into_parent() // Outer::Ascent = RootPath
            }
            ::core::result::Result::Err(root_path) => {
                // No OuterPath. Jump already past Outer.
                if let ::core::option::Option::Some(id) = opt_0 {
                    ::core::iter::Extend::extend(effs, after_child_dropped(id));
                }
                // opt_1 rearm skipped — no path
                // exclusive skipped — no path / claim may already be taken
                root_path
            }
        }
    }
}
```

## Walk: KeyA, Inner jumps to root

```text
Inner exclusive:
  take claim
  peel Inner → Outer → Root
  return Err(root)     // Result<OuterPath, RootPath>::Err

Outer match:
  Err(root_path):
    after_child_dropped(snapped id)
    rearm not run
    exclusive not run
    return root_path
```

## Walk: KeyB (no kill)

```text
Inner: Ok(outer_path)   // into_parent only
Outer match Ok(path):
  after_child_ok, rearm, maybe exclusive
  return path.into_parent() → root
```

## Ordered changes

### P0 — Sink; drop ControlFlow for effects

Effects always extend a sink. Return is path-related, not `Break(Output)`.

### P1 — `Claim` + `Dispatch::Ascent` associated type

Derive emits `type Ascent<'a> = Result<ParentPath, Parent::Ascent>`. Leaf normal path: `Ok(into_parent(path))`. Parent `match`es child.

No hop counters. No `Ascent` struct with depth fields.

### P2 — Exclusive via `claim` + path on Ok arm only

Handlers that do not kill: `fn(ev, &mut P)`. Handlers that kill: consume path, return `(effects, Self::Ascent)` or equivalent sugar.

### F1 — `#[post]` on Ok arm

### F2 — kill peels + `Err` nest (helpers)

### F3 — `#[pre_post]`; dropped arm uses pre-snap

### F4 — `only_if_intact` = Ok-arm-only post

### F5 — reshape: return a different path type inside the doll if needed

## Rules

1. Descent schedules; set final. Ascent runs every scheduled post.
2. Owned path position is the dispatch return value (nested `Result`), not counters.
3. `Ok(parent_path)` = this level still has that path after the child peel. `Err(rest)` = jump; this level has no path.
4. Kill = `into_parent` peels + return the matching `Err` nest. No `invalidate(d)` integer.
5. Posts on Ok get `&mut path`. Posts on Err get pre-snap only.
6. `Claim` is separate; exclusive only when this level owns a path (Ok arm), unless a later rule says otherwise.
7. laserbeam `into_parent` is the only peel.
8. Generate matches expand above.

## Tests

- Normal walk: all `Ok` peels; posts see paths
- Jump past Outer: Outer posts dropped arm; no OuterPath in type on that arm
- Claim trap door
- Doll type depth matches spine depth
- No `ascent_hops` / `invalidation_depth` APIs

## Open

- Sugar for kill handlers so users do not write `Err(Ok(Err(...)))` by hand
- Root `Ascent` = `()` vs `0Path` returned to free `dispatch`
- Derived / enum nodes: same doll relative to their parent path type
- Whether exclusive on Err is always skip (claim already taken) or can run with only a higher path
