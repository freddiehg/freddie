# Invalidation: descent schedules, ascent executes

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post leaf to root. Live hop state is internal; posts receive a frozen snapshot.

## End state (what ships)

### Types (`crates/bind`)

```rust
#[derive(Clone, Copy)]
pub enum Mutation {
    /// Live depth was 0 when this snapshot was taken.
    Intact,
    /// Live depth was > 0 when this snapshot was taken. A deeper exclusive
    /// called `invalidate(N)` covering this hop. Child fields in that zone may
    /// already be gone. Prefer pre carriage over reading the child field.
    MaybeDropped,
}

struct Claimed;

/// Internal to bind dispatch. Not passed to user posts.
pub(crate) struct AscentState {
    invalidation_depth: u32,
    claim: Option<Claimed>,
}

/// Passed to user post functions.
pub struct AscentStateSnapshot {
    mutation: Mutation,
}

impl AscentState {
    pub(crate) fn new() -> Self {
        Self {
            invalidation_depth: 0,
            claim: None,
        }
    }

    pub(crate) fn snapshot(&self) -> AscentStateSnapshot {
        AscentStateSnapshot {
            mutation: if self.invalidation_depth == 0 {
                Mutation::Intact
            } else {
                Mutation::MaybeDropped
            },
        }
    }

    /// `depth = depth.max(d)`. Exclusive / kill helper.
    pub(crate) fn invalidate(&mut self, d: u32) {
        self.invalidation_depth = self.invalidation_depth.max(d);
    }

    /// Framework `into_parent` only, after posts.
    pub(crate) fn step_up(&mut self) {
        self.invalidation_depth = self.invalidation_depth.saturating_sub(1);
    }

    pub(crate) fn claim(&mut self) -> Option<Claimed> {
        match self.claim {
            Some(_) => None,
            None => {
                self.claim = Some(Claimed);
                Some(Claimed)
            }
        }
    }

    pub(crate) fn claimed(&self) -> bool {
        self.claim.is_some()
    }
}

impl AscentStateSnapshot {
    pub fn mutation(&self) -> Mutation {
        self.mutation
    }
}
```

### User signatures

```rust
// pre
fn pre(ev: &E, node: Node<&P, D>) -> T;

// post (pre_post threads T; post alone has no T)
fn post(t: T, node: Node<P, D>, snap: &AscentStateSnapshot) -> (Vec<Effect>, P);
fn post(node: Node<P, D>, snap: &AscentStateSnapshot) -> (Vec<Effect>, P);

// exclusive — schedule token like post; body through run_exclusive
fn exclusive(ev: &E, node: Node<P, D>, state: &mut AscentState) -> (Vec<Effect>, P);
```

`#[bind(t => h)]` schedules like `#[post]` (`noop_pre` token) and runs `run_exclusive(h)`. Not the same signature as a post.

### Attr desugar

```rust
#[pre_post(t => (pre, post))]  // (pre, post)
#[post(t => post)]             // (noop_pre, post)
#[bind(t => h)]                // schedule Some(()); ascent: run_exclusive(h)
#[pre(...)]                    // does not exist
```

```rust
fn noop_pre<E, P, D>(_ev: &E, _node: Node<&P, D>) {}
```

### Helpers

```rust
fn run_post<P>(
    path: P,
    snap: &AscentStateSnapshot,
    body: impl FnOnce(Node<P, ()>, &AscentStateSnapshot) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    body(Node { parent: path, data: () }, snap)
}

fn run_exclusive<P>(
    path: P,
    state: &mut AscentState,
    body: impl FnOnce(Node<P, ()>, &mut AscentState) -> (Vec<Effect>, P),
) -> (P, Vec<Effect>) {
    match state.claim() {
        None => (path, Vec::new()),
        Some(Claimed) => body(Node { parent: path, data: () }, state),
    }
}

fn only_if_intact<P, N>(
    project: impl Fn(&mut P) -> &mut N,
    f: impl FnOnce(&mut N) -> Vec<Effect>,
) -> impl FnOnce(Node<P, ()>, &AscentStateSnapshot) -> (Vec<Effect>, P) {
    move |mut node, snap| {
        let effects = match snap.mutation() {
            Mutation::Intact => f(project(&mut node.parent)),
            Mutation::MaybeDropped => Vec::new(),
        };
        (effects, node.parent)
    }
}
```

### `into_parent`

Before:

```rust
pub fn into_parent(self, sink: &mut Vec<Effect>) -> P {
    let (parent, post_effs) = (self.on_into_parent)(self.parent);
    Extend::extend(sink, post_effs);
    parent
}
// on_into_parent: FnOnce(P) -> (P, Vec<Effect>)
```

After:

```rust
pub fn into_parent(self, sink: &mut Vec<Effect>, state: &mut AscentState) -> P {
    let snap = state.snapshot();
    let (parent, post_effs) = (self.on_into_parent)(self.parent, &snap);
    Extend::extend(sink, post_effs);
    state.step_up();
    parent
}
// on_into_parent: FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>)
```

```rust
fn empty_on_into_parent<P>(parent: P, _snap: &AscentStateSnapshot) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}
```

### `Dispatch`

Before (after P0 batch thread; adjust to whatever is on master when implementing):

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> ControlFlow<(), Self::Path<'a>>
    where
        Self: 'a;
}
```

After:

```rust
pub trait Dispatch<M: Bindings>: Place {
    fn dispatch<'a>(
        path: Self::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> (Self::Path<'a>, AscentState)
    where
        Self: 'a;
}

pub fn dispatch<'a, M, N>(path: N::Path<'a>, event: &M::Event) -> Option<Vec<M::Effect>>
where
    M: Bindings,
    N: Dispatch<M> + 'a,
{
    let mut effs = Vec::new();
    let (_path, state) = <N as Dispatch<M>>::dispatch(path, event, &mut effs);
    if state.claimed() || !effs.is_empty() {
        Some(effs)
    } else {
        None
    }
}
```

### Level order

```text
descent: schedule opt_N
from_fn child; child dispatch → (child_path, state)
into_parent: snap = snapshot(); posts(&snap); step_up()
exclusive if scheduled: run_exclusive (claim; body &mut AscentState)
```

Leaf: `AscentState::new()`; exclusive if any; return `(path, state)`.

### Kill

Exclusive body (same-level path return):

```rust
fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, InnerPath) {
    state.invalidate(2); // N = hops leaf → reshape owner inclusive
    // reshape carrier schedules field replace at owner
    (vec![], node.parent)
}
```

Framework stack still walks. Posts at each hop see `snap.mutation()` from depth at snapshot time.

### DX example

```rust
#[derive(Bind)]
#[node(parent = RootPath)]
#[binds(M)]
#[pre_post(AnyKey => (snap_child_id, after_child))]
#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]
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

fn snap_child_id(ev: &KeyEvent, node: Node<&OuterPath, ()>) -> ChildId {
    node.parent.get().inner.id
}

fn after_child(
    id: ChildId,
    node: Node<OuterPath, ()>,
    snap: &AscentStateSnapshot,
) -> (Vec<M::Effect>, OuterPath) {
    match snap.mutation() {
        Mutation::Intact => {
            let _ = (id, node.parent.get().inner.id);
            (vec![], node.parent)
        }
        Mutation::MaybeDropped => (vec![log_destroyed(id)], node.parent),
    }
}

fn rearm(child: &mut AndReturnHome) -> Vec<MercuryEffect> {
    let (guard, schedule) = arm_return_home();
    child.guard = guard;
    vec![schedule]
}

fn outer_handler(
    ev: &KeyEvent,
    node: Node<OuterPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, OuterPath) { ... }

fn inner_handler(
    ev: &KeyEvent,
    node: Node<InnerPath, ()>,
    state: &mut AscentState,
) -> (Vec<M::Effect>, InnerPath) {
    state.invalidate(2);
    (vec![], node.parent)
}
```

### Generated: Inner

```rust
impl Dispatch<M> for Inner {
    fn dispatch<'a>(
        path: <Inner as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> (<Inner as Place>::Path<'a>, AscentState)
    where
        Self: 'a,
    {
        let opt_0 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(noop_pre(
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

        let mut state = AscentState::new();

        if let ::core::option::Option::Some(()) = opt_0 {
            let ev = /* &KeyEvent from event */;
            let (path, out_effs) = run_exclusive(path, &mut state, |node, state| {
                inner_handler(ev, node, state)
            });
            ::core::iter::Extend::extend(effs, out_effs);
            return (path, state);
        }
        (path, state)
    }
}
```

### Generated: Outer

```rust
impl Dispatch<M> for Outer {
    fn dispatch<'a>(
        mut path: <Outer as Place>::Path<'a>,
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
    ) -> (<Outer as Place>::Path<'a>, AscentState)
    where
        Self: 'a,
    {
        let opt_0 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
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

        let opt_1 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = AnyKey;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(noop_pre(
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

        let opt_2 = if let ::core::option::Option::Some(ev) =
            ::core::convert::TryFrom::try_from(event).ok()
        {
            let trigger = KeyA;
            if ::bind::EventTrigger::is_matching(&trigger, ev) {
                ::core::option::Option::Some(noop_pre(
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

        let inner_path = ::laserbeam::PathMut::from_fn(
            path,
            |p| &mut p.get_mut().inner,
            |p| &p.get().inner,
            move |parent, snap| {
                let mut local = ::std::vec::Vec::new();
                let mut path = parent;
                if let ::core::option::Option::Some(id) = opt_0 {
                    let (p, e) = run_post(path, snap, |node, snap| {
                        after_child(id, node, snap)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                if let ::core::option::Option::Some(()) = opt_1 {
                    let (p, e) = run_post(path, snap, |node, snap| {
                        only_if_intact(|p| &mut p.get_mut().return_home, rearm)(node, snap)
                    });
                    path = p;
                    ::core::iter::Extend::extend(&mut local, e);
                }
                (path, local)
            },
        );

        let (inner_path, mut state) =
            <Inner as ::bind::Dispatch<M>>::dispatch(inner_path, event, effs);

        let mut path = inner_path.into_parent(effs, &mut state);

        if let ::core::option::Option::Some(()) = opt_2 {
            let ev = /* &KeyEvent from event */;
            let (p, e) = run_exclusive(path, &mut state, |node, state| {
                outer_handler(ev, node, state)
            });
            path = p;
            ::core::iter::Extend::extend(effs, e);
        }
        (path, state)
    }
}
```

### Walk (Root ⊃ Outer ⊃ Inner; KeyA; Inner `invalidate(2)`)

```text
DESCENT Outer: opt_0 after_child, opt_1 rearm, opt_2 outer bind
DESCENT Inner: opt_0 inner bind

LEAF Inner:
  state = AscentState::new()
  claim() ok; invalidate(2); return (InnerPath, state)

Outer into_parent:
  snap = snapshot()        // MaybeDropped, depth 2
  after_child(&snap)
  only_if_intact → skip
  step_up()                // depth 1

Outer exclusive: claim fails

Root into_parent (if posts):
  snap = snapshot()        // MaybeDropped, depth 1
  step_up()                // depth 0
```

---

## Ordered changes

Each step ships alone. Behavior-identical until a step says otherwise.

### P0 — `Bindings::Effect` + threaded batch (keep `Break`)

Before: `type Output`; `Break(output)`.

After:

```rust
pub trait Bindings {
    type Trigger: Eq + Hash;
    type Event;
    type Effect;
}

// dispatch(..., effs: &mut Vec<M::Effect>) -> ControlFlow<(), Path>
// exclusive pushes onto effs, Break(())
// top-level Some(effs) / None
```

### P1 — `on_into_parent` + sink together

Before: `into_parent(self) -> P` (or current master shape).

After: `F` on `PathMut`; `into_parent` runs `F` and extends sink. All call sites pass `empty_on_into_parent` (no posts yet). No unused sink alone.

```rust
fn empty_on_into_parent<P>(parent: P) -> (P, Vec<Effect>) {
    (parent, Vec::new())
}
```

(Signature gains `&AscentStateSnapshot` in P3 when state exists; until then match the `F` of that step.)

### P2 — `from_fn` framework-only

Crate-private / sealed.

### P3 — `AscentState` internal + `into_parent` snapshot/step_up

- Leaf: `AscentState::new()`; return `(Path, AscentState)` always (drop `Break`).
- `into_parent(self, sink, state: &mut AscentState)`: `snapshot` → `on_into_parent(..., &snap)` → `step_up`.
- `on_into_parent: FnOnce(P, &AscentStateSnapshot) -> (P, Vec<Effect>)`.
- Exclusive: `run_exclusive` via `claim`; body `(ev, node, &mut AscentState) -> (Vec, P)`; same-level path.
- Top-level: `claimed() || !effs.is_empty()`.
- No user posts yet; posts empty.

### P4 — `#[bind]` only through `run_exclusive`

Handlers return `(Vec<Effect>, P)`. Behavior-identical to P3.

### F1 — `#[post]`

User post `(node, &AscentStateSnapshot) -> (Vec, P)` (or expression form). Generate runs posts inside `on_into_parent` with the snap from `into_parent`.

### F2 — `invalidate(N)` on kill

Exclusive kill helpers call `state.invalidate(N)`. `N` = framework hops from exclusive level to reshape owner (inclusive). Reshape carrier may still be empty; counter + snapshot behavior is testable alone.

### F3 — `#[pre_post]` / `noop_pre`

`#[post]` alone = `(noop_pre, post)`. `#[pre_post]` threads pre return into post first arg.

### F4 — `only_if_intact` + mercury rearm

```rust
#[post(AnyKey => only_if_intact(|p| &mut p.get_mut().return_home, rearm))]
```

### F5 — reshape carrier

Schedule field replace at owner without exclusive consuming `PathMut` / changing return path type. Exclusive still returns same-level `P`.

---

## Rules

1. Descent schedules `opt_0`… only; set is final.
2. Ascent runs every scheduled post.
3. `AscentState` is `pub(crate)`. Posts take `&AscentStateSnapshot` only.
4. `into_parent` = `snapshot` → posts → `step_up`.
5. Kill = `invalidate(N)` on live state; same-level path return; stack still walks.
6. `claim` only in `run_exclusive`.
7. Generate stays thin: schedule + call helpers. Full expand above is the template.
