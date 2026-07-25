# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

## Goal

```rust
path.into_parent().into_parent().complete()
// → AscentOf<OriginPath> : nested Here/Up of path types up the spine
```

Public doll arms are **Here** and **Up**. Not `Ok`/`Err`, not `ControlFlow`.

Deliverable when implemented: unit tests that assert the nest shapes. No dispatch, claim, posts. Minimal peel spine (`Step`) is enough; laserbeam later.

## Public doll

```rust
/// One layer of the leave doll.
pub enum Doll<H, U> {
    Here(H),
    Up(U),
}
```

```text
Here(path)  — stop at this path; posts may use it
Up(rest)    — jumped past; rest is the thinner doll
```

Nested:

```text
Doll<MyPath, Doll<ParentPath, Doll<GrandParentPath, …>>>
```

Name the whole nest by leave origin (path and/or node, as needed) — not by writing the nest out:

```rust
pub type AscentOf<P> = <P as AscentOut>::Out;
// and/or type Ascent<'a> = AscentOf<<Self as Place>::Path<'a>>;
```

## Type family

```rust
pub trait AscentOut {
    type Out;
}

impl AscentOut for Root {
    type Out = Root;
}

/// Here(parent) if stop after one peel; Up(parent’s full doll) if go further.
impl<N, P: AscentOut> AscentOut for Step<N, P> {
    type Out = Doll<P, P::Out>;
}
```

Unrolling (`MyPath = Step<B, Root>`, child = `Step<C, MyPath>`):

```text
Root::Out                   = Root

Step<B, Root>::Out          = Doll<Root, Root>

Step<C, Step<B, Root>>::Out = Doll<Step<B, Root>, Doll<Root, Root>>
                            = Doll<MyPath, Doll<ParentPath, …>>
                            = what post unpacks when C returns to B
```

Yes for every spine path type: `Out = <P as AscentOut>::Out`.
No if the parent chain is not in the type.

Posts unpack the **child’s** `AscentOf<ChildPath>`:

```rust
match child_ascent {
    Doll::Here(my_path) => { /* posts with my_path */ }
    Doll::Up(rest) => { /* posts dropped; rest thinner */ }
}
```

## Minimal peel spine

```rust
pub struct Root;

pub struct Step<Node, Parent> {
    parent: Parent,
    _node: core::marker::PhantomData<Node>,
}

impl<Node, Parent> Step<Node, Parent> {
    pub const fn new(parent: Parent) -> Self {
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

## Pack machine

```rust
use core::marker::PhantomData;

pub trait Pack<P> {
    type Out;
    fn pack(self, path: P) -> Self::Out;
}

/// Stop here → Doll::Here(path).
pub struct AsHere<E>(PhantomData<E>);

impl<P, E> Pack<P> for AsHere<E> {
    type Out = Doll<P, E>;
    fn pack(self, path: P) -> Doll<P, E> {
        Doll::Here(path)
    }
}

/// Layer skipped → Doll::Up(inner.pack(path)).
pub struct AsUp<Q, Inner>(Inner, PhantomData<Q>);

impl<Q, Inner, P> Pack<P> for AsUp<Q, Inner>
where
    Inner: Pack<P>,
{
    type Out = Doll<Q, Inner::Out>;
    fn pack(self, path: P) -> Doll<Q, Inner::Out> {
        Doll::Up(self.0.pack(path))
    }
}

/// Terminal: Out is the path itself (no Doll wrapper).
pub struct AsTerminal;

impl<P> Pack<P> for AsTerminal {
    type Out = P;
    fn pack(self, path: P) -> P {
        path
    }
}

pub trait PeelPack<Node, Parent>: Pack<Step<Node, Parent>> + Sized {
    type After: Pack<Parent, Out = Self::Out>;
    fn peel_pack(self) -> Self::After;
}

impl<Node, Parent, E> PeelPack<Node, Parent> for AsHere<Doll<Parent, E>> {
    type After = AsUp<Step<Node, Parent>, AsHere<E>>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsHere(PhantomData), PhantomData)
    }
}

/// Bare parent rest (root boundary): AsHere<Parent> → Up(parent) via AsTerminal.
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

/// First peel; Rest = Parent::Out so Pack::Out = <Step<N,Parent> as AscentOut>::Out.
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

### Concrete `complete()`

```rust
// Path::complete
let Path { focus, pack } = self;
pack.pack(focus)

// AsHere:   Doll::Here(focus)
// AsUp:     Doll::Up(inner.pack(focus))
// AsTerminal: focus
```

### Worked: two peels → `Up(Here(Root))`

```rust
type BPath = Step<B, Root>;
type CPath = Step<C, BPath>;
// CPath::Out = Doll<BPath, Doll<Root, Root>>

// after_first_peel(c):
//   Path { focus: BPath, pack: AsHere<Doll<Root, Root>> }
// complete → Doll::Here(b)

// .into_parent():
//   Path { focus: Root, pack: AsUp<BPath, AsHere<Root>> }
// complete →
//   AsUp.pack(Root) = Doll::Up(AsHere.pack(Root)) = Doll::Up(Doll::Here(Root))
```

One peel: `Doll::Here(b)`.
Bare rest peel (`AsHere<Root>` on `Step<_, Root>` then `into_parent`): `Doll::Up(Root)`.

## Tests (when implemented)

```text
cargo test -p bind peel_complete   # or wherever it lands
```

| Case | Assert |
| --- | --- |
| one peel from C | `Doll::Here(b)` |
| two peels from C | `Doll::Up(Doll::Here(Root))` |
| one peel from B | `Doll::Here(Root)` |
| bare rest peel | `Doll::Up(Root)` |
| type | result assigns to `AscentOf<CPath>` |

## Ordered changes

### 1 — `Root`, `Step`, `Doll`, `AscentOut`, `AscentOf`

### 2 — `Pack` / `AsHere` / `AsUp` / `AsTerminal` / `PeelPack` / `Path` / `after_first_peel` / `complete`

### 3 — unit tests above

### 4 — (later) same for `laserbeam::PathMut`

## Rules

1. Public arms are `Here` / `Up` only.
2. `Out` for leave at `T` is `<T as AscentOut>::Out`.
3. `complete` = `pack.pack(focus)`; peels rewrite pack, not Out.
4. Name by leave origin path/node as needed; do not spell the nest in signatures.
5. No implementation until the design is settled; deliverable is tests that assert the nests.

## Relation to invalidation

Dispatch leave/kill uses this machine. Parent matches `Doll::Here` / `Doll::Up` (or the same arms on an opaque wrapper). Claim/posts are separate.
