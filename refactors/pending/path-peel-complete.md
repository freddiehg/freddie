# Path peel + complete (nested Result pack)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

```rust
path.into_parent().into_parent().complete()
// → something of a single fixed type Out containing Err(…) / Ok(…) nest
```

Core: unwind peels while wrapping into a nested `Result` doll whose type is known for that path.

No dispatch, claim, posts. Simpler peel spine than laserbeam is fine; same mechanics.

## Do we know the type of `path.ascend_to_root().complete()` for all paths?

**Not from the value alone, and not from a free `Out` parameter you invent at the call site.**
**Yes, if the path’s static type carries a type-level spine** — then `Out` is an associated type of that spine.

| Starting path type (static) | `Out` = type of `ascend_to_root().complete()` |
| --- | --- |
| Root `R` | `R` (already there; identity) |
| `Step<N, P>` peels to `P` | `Result<P, <P as AscentOut>::Out>` |

Recursive definition:

```rust
/// What `ascend_to_root().complete()` returns when leave starts at `Self`.
pub trait AscentOut {
    type Out;
}

// Terminal focus (root of the tree).
impl AscentOut for Root {
    type Out = Root;
}

// One peel layer: Ok(parent) if we stop after one peel; Err(rest) if we go further.
// rest is parent’s full ascent-to-root doll.
impl<N, P: AscentOut> AscentOut for Step<N, P> {
    type Out = Result<P, P::Out>;
}
```

Examples:

```rust
// Root
// Out = Root

// Step<B, Root>
// Out = Result<Root, Root>
//   one peel + complete → Ok(root)
//   (cannot peel past root)

// Step<C, Step<B, Root>>
// Out = Result<Step<B, Root>, Result<Root, Root>>
//   one peel + complete → Ok(b)
//   two peels + complete → Err(Ok(root))   // inner AsHere on Root
//   with bare-terminal packing (AsTerminal) on last layer:
//   Out = Result<Step<B, Root>, Root>
//   two peels + complete → Err(root)
```

So: **for every path type that implements `AscentOut`, yes — `Out` is `<ThatPath as AscentOut>::Out`.**
For an arbitrary unsized/`dyn` path with no spine in the type: **no** — there is nothing to compute `Out` from.

### How we get the type

1. **Type family on the path type** (above): `AscentOut::Out`. This is the answer for all well-typed spines.
2. **Derive / node place** (invalidation later): each node’s laserbeam path type is a known nest of `PathMut` (or `Step`); the derive names `type Ascent = Ascent<<Path as AscentOut>::Out>` or the equivalent `Result` nest. Same computation, fixed per node.
3. **Not** inference from `complete()` alone without `AscentOut` / pack: `complete()` returns `Pk::Out`, and the pack must have been built for that path’s spine. The pack is how the value-level unwind mirrors the type-level doll.

`after_first_peel::<N, P, Rest>` that takes `Rest` by hand is only OK when the caller already knows `Rest = <P as AscentOut>::Out`. Prefer:

```rust
after_first_peel(path)  // Rest inferred as <Parent as AscentOut>::Out
```

## Simpler path (this prefactor)

No projections, no `get_mut`. Only peel:

```rust
/// Minimal spine: node tag + parent path.
pub struct Step<Node, Parent> {
    pub parent: Parent,
    _node: core::marker::PhantomData<Node>,
}

impl<Node, Parent> Step<Node, Parent> {
    pub fn new(parent: Parent) -> Self {
        Self {
            parent,
            _node: core::marker::PhantomData,
        }
    }

    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

laserbeam `PathMut` later: same `into_parent` shape; `AscentOut` + `PeelPack` impls match on `PathMut` instead of `Step`. This prefactor proves the unwind only.

## Pack machine (value-level unwind)

```rust
use core::marker::PhantomData;

pub trait Pack<P> {
    type Out;
    fn pack(self, path: P) -> Self::Out;
}

pub struct AsHere<E>(PhantomData<E>);

impl<P, E> Pack<P> for AsHere<E> {
    type Out = Result<P, E>;
    fn pack(self, path: P) -> Result<P, E> {
        Ok(path)
    }
}

pub struct AsUp<Q, Inner>(Inner, PhantomData<Q>);

impl<Q, Inner, P> Pack<P> for AsUp<Q, Inner>
where
    Inner: Pack<P>,
{
    type Out = Result<Q, Inner::Out>;
    fn pack(self, path: P) -> Self::Out {
        Err(self.0.pack(path))
    }
}

pub struct AsTerminal;

impl<P> Pack<P> for AsTerminal {
    type Out = P;
    fn pack(self, path: P) -> P {
        path
    }
}

/// Rewrite pack when focus peels Step → Parent; Pack::Out unchanged.
pub trait PeelPack<Node, Parent>: Pack<Step<Node, Parent>> + Sized {
    type After: Pack<Parent, Out = Self::Out>;
    fn peel_pack(self) -> Self::After;
}

impl<Node, Parent, E> PeelPack<Node, Parent> for AsHere<Result<Parent, E>> {
    type After = AsUp<Step<Node, Parent>, AsHere<E>>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsHere(PhantomData), PhantomData)
    }
}

impl<Node, Parent> PeelPack<Node, Parent> for AsHere<Parent> {
    type After = AsUp<Step<Node, Parent>, AsTerminal>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsTerminal, PhantomData)
    }
}

impl<Node, Parent, Q, Inner> PeelPack<Node, Parent> for AsUp<Q, Inner>
where
    Inner: PeelPack<Node, Parent>,
{
    type After = AsUp<Q, Inner::After>;
    fn peel_pack(self) -> Self::After {
        AsUp(self.0.peel_pack(), PhantomData)
    }
}

pub struct Path<P, Pk> {
    focus: P,
    pack: Pk,
}

impl<P, Pk> Path<P, Pk>
where
    Pk: Pack<P>,
{
    pub fn complete(self) -> Pk::Out {
        self.pack.pack(self.focus)
    }
}

impl<Node, Parent, Pk> Path<Step<Node, Parent>, Pk>
where
    Pk: PeelPack<Node, Parent>,
{
    pub fn into_parent(self) -> Path<Parent, Pk::After> {
        Path {
            focus: self.focus.into_parent(),
            pack: self.pack.peel_pack(),
        }
    }
}

/// First peel; Rest = <Parent as AscentOut>::Out.
pub fn after_first_peel<Node, Parent>(
    path: Step<Node, Parent>,
) -> Path<Parent, AsHere<Parent::Out>>
where
    Parent: AscentOut,
{
    Path {
        focus: path.into_parent(),
        pack: AsHere(PhantomData),
    }
}
```

Check: `after_first_peel` Out = `Result<Parent, Parent::Out>`.
`AscentOut for Step<Node, Parent>` says `Out = Result<Parent, Parent::Out>`.
**Same type.** First peel’s `complete()` type is exactly `<Step<Node, Parent> as AscentOut>::Out`.

### `ascend_to_root`

```rust
pub trait AscendToRoot: AscentOut + Sized {
    fn ascend_to_root_complete(self) -> Self::Out;
}

impl AscendToRoot for Root {
    fn ascend_to_root_complete(self) -> Root {
        self
    }
}

impl<Node, Parent> AscendToRoot for Step<Node, Parent>
where
    Parent: AscentOut,
    // need value-level peel-all: recursive complete
    Path<Parent, AsHere<Parent::Out>>: AscendContinue<Parent::Out>,
{
    fn ascend_to_root_complete(self) -> Self::Out {
        after_first_peel(self).go_to_root()
    }
}

/// Keep peeling until pack cannot PeelPack (terminal focus), then complete.
pub trait AscendContinue<Out> {
    fn go_to_root(self) -> Out;
}

impl<P, Pk> AscendContinue<Pk::Out> for Path<P, Pk>
where
    Pk: Pack<P>,
{
    default fn go_to_root(self) -> Pk::Out {
        self.complete()
    }
}
```

Specialization/`default` is messy on stable. Prefer **explicit peels** for the prefactor tests, and for `ascend_to_root` a recursive helper trait without specialization:

```rust
pub trait PeelAll {
    type Out;
    fn peel_all_complete(self) -> Self::Out;
}

impl<P, Pk> PeelAll for Path<P, Pk>
where
    Pk: Pack<P>,
{
    type Out = Pk::Out;
    fn peel_all_complete(self) -> Pk::Out {
        self.complete()
    }
}

impl<Node, Parent, Pk> Path<Step<Node, Parent>, Pk>
where
    Pk: PeelPack<Node, Parent>,
{
    /// Peel one step; caller chains or uses a macro to root.
    pub fn into_parent(self) -> Path<Parent, Pk::After> { /* as above */ }
}
```

**Fully typed `ascend_to_root` for all spines** without specialization:

```rust
pub trait AscendToRoot: Sized {
    type Out;
    fn ascend_to_root(self) -> Self::Out;
}

impl AscendToRoot for Root {
    type Out = Root;
    fn ascend_to_root(self) -> Root {
        self
    }
}

impl<Node, Parent> AscendToRoot for Step<Node, Parent>
where
    Parent: AscentOut,
    Path<Parent, AsHere<<Parent as AscentOut>::Out>>: PeelToRoot,
{
    type Out = Result<Parent, <Parent as AscentOut>::Out>;
    fn ascend_to_root(self) -> Self::Out {
        PeelToRoot::peel_to_root(after_first_peel(self))
    }
}

pub trait PeelToRoot: Sized {
    type Out;
    fn peel_to_root(self) -> Self::Out;
}

// Terminal focus: pack is AsHere<...> or AsUp ending at non-Step — just complete.
// Recursive: if focus is Step, into_parent then peel_to_root.

impl<P, E> PeelToRoot for Path<P, AsHere<E>>
where
    AsHere<E>: Pack<P>,
{
    type Out = <AsHere<E> as Pack<P>>::Out;
    fn peel_to_root(self) -> Self::Out {
        self.complete()
    }
}

impl<Node, Parent, E> PeelToRoot for Path<Step<Node, Parent>, AsHere<E>>
where
    AsHere<E>: PeelPack<Node, Parent>,
    Path<Parent, <AsHere<E> as PeelPack<Node, Parent>>::After>: PeelToRoot,
{
    type Out = <Path<Parent, <AsHere<E> as PeelPack<Node, Parent>>::After> as PeelToRoot>::Out;
    fn peel_to_root(self) -> Self::Out {
        self.into_parent().peel_to_root()
    }
}

// Same pattern for Path<Step<...>, AsUp<...>> when focus is still Step:
impl<Node, Parent, Q, Inner> PeelToRoot for Path<Step<Node, Parent>, AsUp<Q, Inner>>
where
    AsUp<Q, Inner>: PeelPack<Node, Parent>,
    Path<Parent, <AsUp<Q, Inner> as PeelPack<Node, Parent>>::After>: PeelToRoot,
{
    type Out = <Path<Parent, <AsUp<Q, Inner> as PeelPack<Node, Parent>>::After> as PeelToRoot>::Out;
    fn peel_to_root(self) -> Self::Out {
        self.into_parent().peel_to_root()
    }
}

// When focus is Root (not Step), only complete impls apply (AsHere/AsUp/AsTerminal on Root).
impl<E> PeelToRoot for Path<Root, AsHere<E>>
where
    AsHere<E>: Pack<Root>,
{
    type Out = <AsHere<E> as Pack<Root>>::Out;
    fn peel_to_root(self) -> Self::Out {
        self.complete()
    }
}

impl<Q, Inner> PeelToRoot for Path<Root, AsUp<Q, Inner>>
where
    AsUp<Q, Inner>: Pack<Root>,
{
    type Out = <AsUp<Q, Inner> as Pack<Root>>::Out;
    fn peel_to_root(self) -> Self::Out {
        self.complete()
    }
}

impl PeelToRoot for Path<Root, AsTerminal> {
    type Out = Root;
    fn peel_to_root(self) -> Root {
        self.complete()
    }
}
```

Overlap: `Path<Step<...>, AsHere<E>>` vs `Path<P, AsHere<E>>` — more specific Step impl peels; generic complete-only for non-Step. On stable, avoid the blanket `Path<P, AsHere<E>>` complete and only list terminal foci (`Root`) plus `Step` recurse.

**Bottom line for the type question:**

```rust
// For any Step-spine path type T: AscentOut + AscendToRoot
let out: <T as AscentOut>::Out = t.ascend_to_root();
// or peel_to_root after after_first_peel — same Out
```

We know the type **because** it is `<T as AscentOut>::Out`, defined by induction on the path type. We do not get it by “running” peels in the typechecker without that trait (or an equivalent derive-written alias).

## Worked values

```rust
struct Root;
struct B;
struct C;

type BPath = Step<B, Root>;
type CPath = Step<C, BPath>;

// <CPath as AscentOut>::Out = Result<BPath, Result<Root, Root>>
//   (with Parent::Out for Root = Root)

let c = Step::<C, _>::new(Step::<B, _>::new(Root));

let out = after_first_peel(c).peel_to_root();
// one peel to B then peel_to_root peels B→Root then complete
// → Err(Ok(Root)) or Err(Root) depending on bare vs Result<Root,Root>
```

Bare root rest (prefer for real trees):

```rust
impl AscentOut for Root {
    type Out = Root;
}

// Step<N, Root>::Out = Result<Root, Root>  // still double Root
// Optional nicety later: trait AscentOut { type Out; type Rest; }
// Step::Out = Result<Parent, Parent::Rest>, Root::Rest = ! or Root with AsTerminal only
```

Keep `Result<Root, Root>` for the prefactor if it compiles; invalidation can opaque-wrap.

## Tests

```rust
#[test]
fn ascent_out_matches_complete_type() {
    // type equality via assigning to annotated let
    let c: CPath = /* ... */;
    let out: <CPath as AscentOut>::Out = after_first_peel(c).into_parent().complete();
    let _ = out;
}

#[test]
fn two_peels_err_ok() {
    let out = after_first_peel(c).into_parent().complete();
    assert!(matches!(out, Err(Ok(Root))));
}

#[test]
fn one_peel_ok() {
    let out = after_first_peel(c).complete();
    assert!(matches!(out, Ok(_)));
}
```

## Ordered changes

### 1 — `Step`, `AscentOut`

### 2 — `Pack` / `AsHere` / `AsUp` / `AsTerminal` / `PeelPack` / `Path`

### 3 — `after_first_peel` with `Rest = Parent::Out`

### 4 — `PeelToRoot` / `ascend_to_root` (optional sugar; tests can chain `into_parent`)

### 5 — unit tests; type assertion `<T as AscentOut>::Out`

### 6 — (later) impl `AscentOut` + `PeelPack` for `laserbeam::PathMut` same as `Step`

## Rules

1. `Out` for a leave starting at `T` is `<T as AscentOut>::Out` when `T` is a spine type.
2. Without a spine in the type, `Out` is unknown — pass it in only if something else (derive) computed it.
3. Value unwind: pack peels with `into_parent` / `peel_pack`; `complete` = `pack.pack(focus)`.
4. `after_first_peel` must use `Parent::Out`, not a free `Rest` type parameter, so the type matches `AscentOut`.

## Relation to invalidation

Dispatch leave/kill calls this machine; wraps `Out` in opaque `Ascent`. Per-node path types already nest parents — they implement `AscentOut` the same way.
