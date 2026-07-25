# Path peel + complete (nested Result pack)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

```rust
path.into_parent().into_parent().complete()
// → something of a single fixed type Out containing Err(…) / Ok(…) nest
```

Core: unwind peels while wrapping into a nested `Result` doll whose type is known for that path.

No dispatch, claim, posts. Simpler peel spine than laserbeam is fine; same mechanics.

## The type posts need

When a child leaves, the parent runs posts. The parent must know the type of the value the child returned:

```text
Result<MyPath, Result<ParentPath, Result<GrandParentPath, …>>>
```

That is the child’s leave doll. From child path `Step<Child, MyPath>` (parent focus = `MyPath`):

```text
<Step<Child, MyPath> as AscentOut>::Out
  = Result<MyPath, <MyPath as AscentOut>::Out>
  = Result<MyPath, Result<ParentPath, Result<GrandParentPath, …>>>
```

Unpack:

```text
Ok(my_path)  →  Here — posts run with MyPath
Err(rest)    →  Up   — rest : Result<ParentPath, Result<GrandParentPath, …>>
```

After Here posts, this node leaves with one thinner doll:

```text
<MyPath as AscentOut>::Out
  = Result<ParentPath, Result<GrandParentPath, …>>
```

Same type family; posts need the **child’s** `AscentOut::Out` as the type of what they unpack.

## Do we know that type for all paths?

**Yes**, if the path type is a spine (`Root`, `Step<N, P>`, later `PathMut` nest): it is `<Path as AscentOut>::Out`.

**No**, if the parent chain is not in the type — there is nothing to form the nest from.

### Type family

```rust
/// Doll returned when leave starts at this path type.
pub trait AscentOut {
    type Out;
}

impl AscentOut for Root {
    type Out = Root;
}

/// Ok(parent) if stop after one peel; Err(parent’s full doll) if go further.
impl<N, P: AscentOut> AscentOut for Step<N, P> {
    type Out = Result<P, P::Out>;
}
```

Unrolling (`MyPath = Step<B, Root>`, child = `Step<C, MyPath>`):

```text
Root::Out                              = Root

Step<B, Root>::Out                     = Result<Root, Root>
                                       // = Result<ParentPath, …> for B

Step<C, Step<B, Root>>::Out            = Result<Step<B, Root>, Result<Root, Root>>
                                       // = Result<MyPath, Result<ParentPath, …>>
                                       // = type of child return / what post unpacks at B
```

### How we get the type

1. Associated type: `<ChildPath as AscentOut>::Out` (unpack / posts), `<MyPath as AscentOut>::Out` (my leave after posts).
2. Derive: each `Place::Path` is already a known nest; emit the same `Result` chain (or `type Ascent<'a> = …` alias). No runtime.
3. Not a free type parameter on the post function, and not inferred without the spine.

Value-level pack/`complete` must return exactly that associated type (`after_first_peel` sets `Rest = Parent::Out` so `Pack::Out = <Step<N,Parent> as AscentOut>::Out`).

### How we name it

Do **not** name the expanded `Result<…, Result<…, …>>` nest in signatures.

Name the doll by the leave origin — the **path** type, and/or the **node** (`Place`) that owns that path — whichever we actually need at the call site. Same type either way (`Place::Path` ties them).

```rust
// The type is always <OriginPath as AscentOut>::Out
// Surface names we may use (only as needed):

pub type AscentOf<P> = <P as AscentOut>::Out;

// and/or on the node, if Dispatch lives on Place:
// type Ascent<'a> = AscentOf<<Self as Place>::Path<'a>>;

// and/or a newtype if the Result should stay private:
// pub struct Ascent<P: AscentOut> { doll: P::Out }
```

Do not ship both path-keyed and node-keyed public aliases until a call site forces it. Prefer one name in code; the other is just `AscentOf` of the related path/node.

Unpack `Here` / `Up` uses that name, not the raw nest.

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
    /// Finish the leave at the current focus. Runs the pack stack only — no further peels.
    pub fn complete(self) -> Pk::Out {
        let Path { focus, pack } = self;
        pack.pack(focus)
    }
}

impl<Node, Parent, Pk> Path<Step<Node, Parent>, Pk>
where
    Pk: PeelPack<Node, Parent>,
{
    pub fn into_parent(self) -> Path<Parent, Pk::After> {
        let Path { focus, pack } = self;
        Path {
            focus: focus.into_parent(),
            pack: pack.peel_pack(),
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

### Concrete `complete()` (what actually runs)

`Path::complete` is only:

```rust
pub fn complete(self) -> Pk::Out {
    let Path { focus, pack } = self;
    pack.pack(focus)
}
```

All nesting is in `Pack::pack`. Monomorphized bodies:

```rust
// --- AsHere<E>: stop here ---
impl<P, E> Pack<P> for AsHere<E> {
    type Out = Result<P, E>;
    fn pack(self, path: P) -> Result<P, E> {
        Ok(path)
    }
}

// Path<P, AsHere<E>>::complete()
//   = AsHere.pack(focus)
//   = Ok(focus)

// --- AsUp<Q, Inner>: this layer was skipped ---
impl<Q, Inner, P> Pack<P> for AsUp<Q, Inner>
where
    Inner: Pack<P>,
{
    type Out = Result<Q, Inner::Out>;
    fn pack(self, path: P) -> Result<Q, Inner::Out> {
        let AsUp(inner, _) = self;
        Err(inner.pack(path))
    }
}

// Path<P, AsUp<Q, Inner>>::complete()
//   = Err(inner.pack(focus))
//   and inner.pack is again AsHere / AsUp / AsTerminal

// --- AsTerminal: bare path is the whole Out ---
impl<P> Pack<P> for AsTerminal {
    type Out = P;
    fn pack(self, path: P) -> P {
        path
    }
}

// Path<P, AsTerminal>::complete()
//   = focus
```

### Worked: two peels then `complete` → `Err(Ok(a))`

Spine: `CPath = Step<C, Step<B, A>>`, `A` terminal with `AscentOut::Out = A` (treat `A` like a root token for this example).

```rust
type BPath = Step<B, A>;
type CPath = Step<C, BPath>;
// <CPath as AscentOut>::Out = Result<BPath, Result<A, A>>
//   (if A::Out = A)

let c: CPath = Step::new(Step::new(a));

// after_first_peel(c):
//   focus = BPath
//   pack  = AsHere::<Result<A, A>>   // Parent::Out = Result<A, A> when A::Out = A
//   type  = Path<BPath, AsHere<Result<A, A>>>

// Prefer Rest = A with bare terminal for clarity of Err(Ok) vs Err:
// <BPath as AscentOut>::Out = Result<A, A> if A::Out = A
// For bare-style Rest = A on first peel from C when BPath peels to A:
// after_first_peel with Parent = BPath, Parent::Out = Result<A, A>
```

Cleaner spine `C → B → Root` with `Root::Out = Root`:

```rust
type BPath = Step<B, Root>;
type CPath = Step<C, BPath>;
// CPath::Out = Result<BPath, Result<Root, Root>>

let c = Step::<C, _>::new(Step::<B, _>::new(Root));

// 1) after_first_peel(c)
let p0: Path<BPath, AsHere<Result<Root, Root>>> = Path {
    focus: c.into_parent(),              // BPath
    pack: AsHere(PhantomData),         // Rest = BPath::Out = Result<Root, Root>
};
// p0.complete() would be:
//   AsHere.pack(b_path) = Ok(b_path)
//   : Result<BPath, Result<Root, Root>>

// 2) p0.into_parent()
//    focus BPath = Step<B, Root> → Root
//    pack AsHere<Result<Root, Root>> peels via PeelPack:
//      After = AsUp<BPath, AsHere<Root>>
let p1: Path<Root, AsUp<BPath, AsHere<Root>>> = Path {
    focus: p0.focus.into_parent(),       // Root
    pack: AsUp(AsHere(PhantomData), PhantomData),
};

// 3) p1.complete() — concrete expansion:
pub fn complete(self) -> Result<BPath, Result<Root, Root>> {
    // self: Path<Root, AsUp<BPath, AsHere<Root>>>
    let Path { focus, pack } = self;
    // pack: AsUp<BPath, AsHere<Root>>
    // focus: Root
    pack.pack(focus)
}

// pack.pack(focus) for AsUp:
fn pack(self, path: Root) -> Result<BPath, Result<Root, Root>> {
    let AsUp(inner, _) = self;           // inner: AsHere<Root>
    Err(inner.pack(path))
}

// inner.pack for AsHere<Root>:
fn pack(self, path: Root) -> Result<Root, Root> {
    Ok(path)
}

// so complete() = Err(Ok(Root))
```

One peel only:

```rust
// p0.complete():
fn complete(self) -> Result<BPath, Result<Root, Root>> {
    let Path { focus, pack } = self;     // focus: BPath, pack: AsHere<Result<Root, Root>>
    pack.pack(focus)                     // Ok(focus)
}
// = Ok(b_path)
```

Bare rest (`after_first_peel` with `Parent::Out = Root` when `BPath = Step<B, Root>` and we use `Out = Result<BPath, Root>` — requires `AscentOut` form `Result<P, P::Out>` with `Root::Out = Root` giving `Result<Root, Root>` for B; for true bare `Result<BPath, Root>` see optional `Rest` later). With current `AscentOut`:

```rust
// two peels from C, complete at Root with nested Result<Root, Root> inner:
// complete() = Err(Ok(Root)) as above
```

Kill-style bare `Err(root)` uses `AsHere<Root>` on focus `Step<_, Root>` then one peel:

```rust
// Path at BPath = Step<B, Root>, pack AsHere<Root>
// into_parent → Path<Root, AsUp<BPath, AsTerminal>>
// complete():
fn complete(self) -> Result<BPath, Root> {
    let Path { focus, pack } = self;     // focus: Root, pack: AsUp<BPath, AsTerminal>
    pack.pack(focus)
}
// AsUp::pack:
//   Err(AsTerminal.pack(Root)) = Err(Root)
```

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
    pub fn into_parent(self) -> Path<Parent, Pk::After> {
        let Path { focus, pack } = self;
        Path {
            focus: focus.into_parent(),
            pack: pack.peel_pack(),
        }
    }
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
