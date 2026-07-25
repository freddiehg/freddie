# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post. Where the owned path sits is the value dispatch returns — not hop counters.

## Model

Spine `A → B → C`. Descent builds a path to `C`.

`C::dispatch` **always** returns `C`’s ascent type (same type on every exit). That type is a private nested doll inside bind. App code never names the `Result` nest.

```text
// bind-private doll for C (illustration)
Result<BPath, Result<APath, …>>

// after peels + complete, same return type every time:
Ok(b)              stopped at B
Err(Ok(a))         stopped at A
Err(Err(…))        stopped further up
```

### The bug with `Return<P>`

`Return<P>` / “marker that is just the path” is the wrong return type. After two peels you hold `PathA`, but `C::dispatch` must still return **C’s ascent**, not `Return<PathA>`. Those are different types. The path at the stop level has to be **wrapped back up** to the origin node’s ascent type.

### `into_parent` installs a wrap; `complete` runs the stack

Leaving a node is a climb: each `into_parent` peels the path **and** pushes a pack callback for that level. `complete()` packages the path you still hold by running those callbacks from the stop level back to the origin. The result type is always the origin’s ascent.

```text
A → B → C
start at C (origin; return type = C’s ascent)

into_parent  →  path = B, stack = [C’s wrap]
into_parent  →  path = A, stack = [C’s wrap, B’s wrap]

complete at A  (apply wraps back to C):
  base at A, then B’s wrap, then C’s wrap
```

What each wrap does (private doll = nested `Result`):

```text
// Stop at this level’s path → Ok(path) for this layer
// Stop further up (got rest from below) → Err(rest) for this layer
```

Worked with path value `PathA` after two peels from `C`:

```text
// conceptual: complete running the stack upward

// after applying only the wrap that sits at A (terminal stop as Err payload
// in the illustration the user cares about for a jump):
complete through A  →  Err(PathA)

// then B’s wrap adds another Err:
complete through B  →  Err(Err(PathA))

// C’s wrap would add another layer if the origin were above C, etc.
```

Same climb, stop at `B` instead (one peel only):

```text
into_parent  →  path = B, stack = [C’s wrap]
complete at B  →  Ok(PathB)   // C’s ascent: Here at B
```

So:

```rust
// always returns C’s ascent type (opaque public type; Result private)
path_c
    .into_parent()   // Climb at B, pack stack includes C
    .into_parent()   // Climb at A, pack stack includes C then B
    .complete()      // PathA wrapped back to C’s ascent
```

Not:

```rust
// WRONG — different type depending on stop path; does not wrap back to C
path.into_parent().into_parent().complete()  // as Return<PathA>
```

### Public API shape

```rust
// Climb is tied to origin ascent type Out (the type dispatch must return).
struct Climb<P, Out> { /* path + pack stack; fields private */ }

// Peel one level: new path is parent; push this level’s wrap into the stack.
impl<Node, Parent, Out> Climb<PathMut<Node, Parent>, Out> {
    fn into_parent(self) -> Climb<Parent, Out>;
}

// Stop here: run pack stack → Out (always origin ascent).
impl<P, Out> Climb<P, Out> {
    fn complete(self) -> Out;
}
```

Starting a climb from the path `dispatch` holds (origin = this node’s ascent):

```rust
// bind, used by derive / handlers that leave or kill
fn climb<P, Out>(path: P) -> Climb<P, Out>
where
    // P packs into Out when completed at P, etc.
;
```

Handler / leaf leave:

```rust
// Normal leave C: one peel, complete → Ok(b) inside Out
climb(path).into_parent().complete()

// Jump to A: two peels, complete → Err(…) inside same Out
climb(path).into_parent().into_parent().complete()
```

`Out` is inferred as `Self::Ascent` from the return position of `dispatch`. Same type every branch.

### Pack stack (bind-private)

Each `into_parent` from a `PathMut` level pushes: “if the stop path is my parent path, this layer is `Ok(parent)`; if the stop is further up, this layer is `Err(rest)`.”

```text
// illustration only — Result is private to bind

// Layer for C (C::Ascent = Result<BPath, B::Ascent>):
//   complete at B:  Ok(b)
//   complete above: Err(b_ascent)

// Layer for B (B::Ascent = Result<APath, A::Ascent>):
//   complete at A:  Ok(a)
//   complete above: Err(a_ascent)

// Climb C → B → A, complete at A:
//   base: a
//   B layer: Ok(a)
//   C layer: Err(Ok(a))
```

User-facing picture of the jump (Err nest growing as wraps apply upward), with stop path `PathA`:

```text
after A’s layer   Err(PathA)           // or Ok(PathA) if A is Here of that layer
after B’s layer   Err(Err(PathA))      // one more Err from B when jump skipped B’s Here
```

Exact `Ok` vs `Err` at the terminal layer follows the private doll (`Ok` when the stop path **is** that layer’s Here path). The important mechanical fact: **wraps run on the way back up; return type is always origin `Out`.**

### One level up (parent)

Parent does not build a climb from the child’s deep path. Child already returned `Child::Ascent` (fully wrapped to the child origin). Parent unpacks one private layer:

```rust
match unpack(child_ascent) {
    Here(mut path) => {
        // posts + exclusive at this path
        climb(path).into_parent().complete() // Out = Parent::Ascent
        // or equivalent: pack Here peel without a multi-stack
    }
    Up(rest) => {
        // posts dropped; rest already Parent::Ascent’s Up payload
        rest
    }
}
```

`unpack` / `Here` / `Up` live in bind. App code does not match `Result`.

### Claim

Separate root-owned slot. Not part of the climb / doll.

### Posts

- `Here`: posts get `&mut path`.
- `Up`: pre-snap only; no path at this level.

## Types (`crates/bind`)

```rust
/// Opaque ascent out of a node. Always the same type for that node.
/// Contains the private doll. No public Result.
pub struct Ascent<D> {
    doll: D, // private; D is the nested Result (or equivalent) layout
}

/// Climb: current path + pack stack back to origin ascent Out.
pub struct Climb<P, Out> {
    path: P,
    // pack stack: private; type-state or private trait object internal
    _out: core::marker::PhantomData<fn() -> Out>,
}

impl<Node, Parent, Out> Climb<laserbeam::PathMut<Node, Parent>, Out> {
    /// Peel one path layer; push this level’s wrap for complete().
    pub fn into_parent(self) -> Climb<Parent, Out>;
}

impl<P, Out> Climb<P, Out> {
    /// Run the pack stack. Type is always Out (origin ascent).
    pub fn complete(self) -> Out;
}

/// Start a climb whose complete() type is Out (usually Self::Ascent).
pub fn climb<P, Out>(path: P) -> Climb<P, Out>;

/// One unpack step for the derive (not public Result).
pub enum Step<Here, Up> {
    Here(Here),
    Up(Up),
}

// bind-private:
// fn unpack_step(...) -> Step<...>;
// pack stack: Here = Ok(path), Up = Err(rest) for each Result layer

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
```

Dispatch:

```rust
pub trait Dispatch<M: Bindings>: Place {
    /// Opaque. Same type on every exit from this node.
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

Derive sets `type Ascent<'a> = Ascent</* private doll type for this node */>`.

## User signatures

```rust
fn pre(ev: &SourceEvent, node: Node<&P, D>) -> T;

fn post_here(pre_return: T, path: &mut P) -> Vec<M::Effect>;
fn post_up(pre_return: T) -> Vec<M::Effect>;

// Leave or kill: climb peels + complete → node’s Ascent (same type always)
fn exclusive_leave(ev: &SourceEvent, path: P) -> (Vec<M::Effect>, NodeAscent);
```

```rust
// Inner kill to root — return type is Inner::Ascent, not Return<RootPath>
fn inner_handler(ev: &KeyEvent, path: InnerPath) -> (Vec<DemoEffect>, <Inner as Dispatch<M>>::Ascent<'_>) {
    let ascent = climb(path).into_parent().into_parent().complete();
    (vec![], ascent)
}
```

## Level order

```text
DESCENT: schedule opts

if child:
  child_path = PathMut::from_fn(path, ...)
  match unpack(Child::dispatch(child_path, event, effs, claim)) {
    Here(path) => {
      // posts + exclusive
      climb(path).into_parent().complete()   // Parent::Ascent
    }
    Up(rest) => {
      // posts dropped
      rest
    }
  }

if leaf:
  exclusive may climb(path).into_parent()…complete()
  else climb(path).into_parent().complete()
```

## DX example

```rust
// Inner::Ascent — always the same opaque type (doll: Result<OuterPath, RootPath>)
// Outer::Ascent — always RootPath (boundary bare / opaque wrap)

fn inner_handler(
    _ev: &KeyEvent,
    path: InnerPath,
) -> (Vec<DemoEffect>, <Inner as Dispatch<M>>::Ascent<'_>) {
    // two peels to root; complete wraps back to Inner::Ascent (Err(root) private)
    (vec![], climb(path).into_parent().into_parent().complete())
}

// normal leave (generated if no exclusive kill):
// climb(path).into_parent().complete()  → Here(outer) inside Inner::Ascent
```

Outer unpack child:

```rust
match unpack(Inner::dispatch(...)) {
    Step::Here(mut outer_path) => {
        // after_child_ok, rearm, exclusive…
        climb(outer_path).into_parent().complete() // Outer::Ascent
    }
    Step::Up(rest) => {
        // after_child_dropped
        rest
    }
}
```

## Walk: A→B→C, kill to A

```text
C exclusive:
  climb(path_c)
    .into_parent()    // path B; C wrap pushed
    .into_parent()    // path A; B wrap pushed
    .complete()       // wrap back to C::Ascent
                      // private: Err(Ok(a)) or Err-nest as per doll
  return that (type C::Ascent)

B unpack:
  Up(...): no B path; dropped posts; pass rest (type B::Ascent)

A unpack:
  Here or Up depending on doll; …
```

## Walk: normal leave C

```text
climb(path_c).into_parent().complete()  // Ok(b) inside C::Ascent
B unpack Here(b): posts; climb(b).into_parent().complete()
```

## Ordered changes

### P0 — Sink; drop ControlFlow

### P1 — `Climb<P, Out>` + `into_parent` pack stack + `complete() -> Out` + private doll + `unpack`/`Step`

No `Return<P>` as dispatch return. No hop counters. No public `Result`.

### P2 — Exclusive / leave via climb peels + complete

### F1 — posts on Here / Up

### F2 — multi-peel kill = more `into_parent` before `complete`

### F3 — pre_post pre-snap for Up

### F4 — only_if_intact = Here only

## Rules

1. Descent schedules; set final. Ascent runs every scheduled post.
2. Each node’s `dispatch` always returns that node’s `Ascent` type.
3. Climb: `into_parent` peels path and pushes a wrap; `complete` runs wraps back to origin `Out`.
4. Private doll (`Result` nest) only inside bind. App never constructs or matches it.
5. Parent: `unpack` → Here (posts + climb peel complete) / Up (posts dropped + pass rest).
6. `Claim` separate. laserbeam peels the path; bind owns the pack stack.
7. No `Return<P>` as the ascent type. No `Infallible`. No hop counters.

## Tests

- Normal leave: Here chain; complete type equals `Self::Ascent`
- Two peels + complete: still `Self::Ascent`, not `Return<PathA>`
- Up arm has no path at skipped level
- App cannot match ascent as `Result`
- Claim trap door

## Open

- Type-state pack stack vs private sealed trait `Pack<Out>` per spine level (must compile; no coherence dodge)
- Whether `Climb::into_parent` is only for `PathMut` or also other `HasParent` paths
- Derived / enum nodes
- Root boundary: `Out` bare path vs `Ascent` newtype around path
