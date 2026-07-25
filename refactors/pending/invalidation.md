# Invalidation: descent schedules, ascent returns a path doll

Not done. Standalone.

Descent schedules which pre/posts/binds run. That set is final. Ascent runs every scheduled post. Where the owned path sits is the value dispatch returns — not hop counters.

## Model

Spine `A → B → C`. Descent builds a path to `C`.

`C::dispatch` **always** returns `C`’s ascent type (same type on every exit). That type is a private nested doll inside bind. App code never names the `Result` nest.

### Path is generic over what `complete()` returns

The path value carries a type parameter `Out`: the type of `complete()`. That is the origin node’s ascent type for this climb. `into_parent` peels the focus and **preserves `Out`**. Every stop still completes to the same `Out`.

```rust
// Node, Parent: laserbeam focus (as today)
// Out: what complete() returns — origin ascent for this climb
PathMut<Node, Parent, Out>
// or a bind Path wrapper: Path<Inner, Out> with the same idea
```

```rust
impl<Node, Parent, Out> PathMut<Node, Parent, Out> {
    /// Peel one layer. Parent path still completes to the same Out.
    pub fn into_parent(self) -> Parent
    where
        Parent: /* still tagged with Out — see below */;

    /// Package this path into Out by running the pack stack pushed on each peel.
    pub fn complete(self) -> Out;
}
```

Parent after peel is not a bare `Parent` that forgot `Out`. The parent path type is still “path at parent focus, complete → Out”. So either:

- `Parent` is itself a `PathMut<…, Out>` (nested paths all share `Out`), and the root of the climb is e.g. `PathMut<C, PathMut<B, PathMut<A, Root, Out>, Out>, Out>` — awkward repeating `Out`, or
- bind’s path type is `Path<P, Out>` where `P` is the laserbeam focus chain and `Out` sits once on the outside:

```rust
pub struct Path<P, Out> {
    focus: P, // PathMut chain or &mut Root
    // pack stack for peels; private
    _out: PhantomData<fn() -> Out>,
}

impl<P, Out> Path<P, Out> {
    pub fn into_parent(self) -> Path<P::Parent, Out>
    where
        P: HasParent;

    pub fn complete(self) -> Out;
}
```

Same fact either way: **`Out` is a generic on the path you hold; `into_parent` does not change `Out`; `complete() -> Out`.**

At `C::dispatch`, the path is `Path<…, C::Ascent>`. After two peels you hold path-at-A still with `Out = C::Ascent`. `complete()` returns `C::Ascent`, not something parameterized by A.

### Climb: peels push wraps; complete runs them

Each `into_parent` peels the focus **and** pushes a pack callback for that level. `complete()` runs the stack from the stop level back to the origin. Return type is always `Out`.

```text
A → B → C
path at C with Out = C::Ascent

into_parent  →  path at B, Out = C::Ascent, stack = [C wrap]
into_parent  →  path at A, Out = C::Ascent, stack = [C wrap, B wrap]

complete() at A → wrap back to C::Ascent
```

Private doll layers (illustration):

```text
// stop at this level’s path → Ok(path) for this layer
// stop further up           → Err(rest) for this layer
```

With stop path `PathA` after two peels from `C`:

```text
through A’s layer  →  Err(PathA)        // jump form as discussed
through B’s layer  →  Err(Err(PathA))
// type is still Out = C::Ascent
```

One peel only, stop at B:

```text
into_parent  →  path at B, Out = C::Ascent
complete()   →  Ok(PathB) inside C::Ascent
```

```rust
// always Out = C::Ascent
path_c
    .into_parent()   // Path at B, same Out
    .into_parent()   // Path at A, same Out
    .complete()      // -> C::Ascent
```

### One level up (parent)

Child returned `Child::Ascent` (already completed to the child’s origin). Parent unpacks one private layer. On `Here`, the recovered path is re-tagged / is a `Path<…, Parent::Ascent>` for the parent’s own leave:

```rust
match unpack(child_ascent) {
    Here(mut path) => {
        // path: Path<…, Parent::Ascent>  (Out = parent ascent)
        // posts + exclusive
        path.into_parent().complete()
    }
    Up(rest) => {
        // posts dropped
        rest // already Parent::Ascent
    }
}
```

`unpack` / `Here` / `Up` in bind only. No public `Result`.

### Claim

Separate root-owned slot. Not on the path.

### Posts

- `Here`: `&mut` focus path.
- `Up`: pre-snap only.

## Types

```rust
/// Path with pack stack. Out = complete() return type (origin ascent).
pub struct Path<P, Out> {
    focus: P,
    // private pack stack
    _out: core::marker::PhantomData<fn() -> Out>,
}

impl<P, Out> Path<P, Out>
where
    P: HasParent,
{
    /// Peel focus; keep Out; push this level’s wrap.
    pub fn into_parent(self) -> Path<P::Parent, Out>;

    pub fn get(&self) -> &P::Node { /* via focus */ }
    pub fn get_mut(&mut self) -> &mut P::Node { /* via focus */ }
}

impl<P, Out> Path<P, Out> {
    /// Run pack stack → always Out.
    pub fn complete(self) -> Out;
}

/// Build path at this node for dispatch. Out = this node’s Ascent.
pub fn path_for<P, Out>(focus: P) -> Path<P, Out>;

pub enum Step<Here, Up> {
    Here(Here),
    Up(Up),
}

pub struct Claim<'c> {
    slot: &'c mut Option<()>,
}

// Claim methods as before (try_take, with_exclusive, …)
```

laserbeam can stay `PathMut<Node, Parent>` as the **focus** only. Bind’s `Path<P, Out>` adds `Out` and the pack stack. Alternatively laserbeam gains `Out` on `PathMut` if we want one type — decision: prefer bind wrapper so laserbeam stays path-only; `Out` is an ascent concern.

Dispatch:

```rust
pub trait Dispatch<M: Bindings>: Place {
    type Ascent<'a>
    where
        Self: 'a;

    fn dispatch<'a, 'c>(
        path: Path<Self::Path<'a>, Self::Ascent<'a>>,
        // or Place::Path already includes Out = Ascent
        event: &M::Event,
        effs: &mut Vec<M::Effect>,
        claim: &mut Claim<'c>,
    ) -> Self::Ascent<'a>
    where
        Self: 'a;
}
```

Cleaner: **`Place::Path<'a>` is already `Path<Focus, Self::Ascent<'a>>`** so the path’s `Out` is this node’s ascent by construction.

## User signatures

```rust
fn exclusive(
    ev: &SourceEvent,
    path: Path<Focus, NodeAscent>,
) -> (Vec<M::Effect>, NodeAscent) {
    let ascent = path.into_parent().into_parent().complete(); // NodeAscent
    (vec![], ascent)
}
```

## Level order

```text
DESCENT: schedule; child PathMut focus with Out = Child::Ascent

if child:
  match unpack(Child::dispatch(child_path, ...)) {
    Here(path) => { /* Out = Parent::Ascent */ path.into_parent().complete() }
    Up(rest) => rest
  }

if leaf:
  path.into_parent().…complete()  // Out = Leaf::Ascent
```

## DX

```rust
fn inner_handler(
    _ev: &KeyEvent,
    path: Path<InnerFocus, <Inner as Dispatch<M>>::Ascent<'_>>,
) -> (Vec<DemoEffect>, <Inner as Dispatch<M>>::Ascent<'_>) {
    (vec![], path.into_parent().into_parent().complete())
}
```

## Walk: two peels to A from C

```text
path: Path<focus_C, C::Ascent>
into_parent → Path<focus_B, C::Ascent>
into_parent → Path<focus_A, C::Ascent>
complete()  → C::Ascent   // Err(Err(PathA)) private nest, same Out
```

## Ordered changes

### P0 — Sink; drop ControlFlow

### P1 — `Path<P, Out>` (or `PathMut<…, Out>`): `into_parent` keeps `Out`, `complete() -> Out`, private pack stack + doll

### P2 — Dispatch path is `Path<_, Self::Ascent>`; exclusive leave via peels + complete

### F1–F4 — posts Here/Up, pre_post, only_if_intact, kill multi-peel

## Rules

1. Descent schedules; set final. Ascent runs every scheduled post.
2. Path is generic over `Out` = what `complete()` returns (origin ascent).
3. `into_parent` peels focus, preserves `Out`, pushes a pack wrap.
4. `complete() -> Out` always — same type at every stop depth.
5. Private `Result` doll only in bind. No public `Result` ascent. No `Return<P>`. No hop counters.
6. Parent: unpack Here/Up; Here path has `Out = Parent::Ascent`.
7. `Claim` separate.

## Tests

- After N peels, path type still has same `Out`
- `complete()` type equals origin `Ascent`, not the stop focus type
- Two peels from C: private nest jump; type is `C::Ascent`
- App cannot match ascent as `Result`

## Open

- Bind `Path<P, Out>` wrapper vs `Out` on laserbeam `PathMut`
- Type-state pack stack representation
- Derived / enum nodes
- Root: `Out` at root free-dispatch boundary
