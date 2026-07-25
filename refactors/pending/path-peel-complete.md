# Path peel + complete (Here / Up doll)

Not done. Standalone. Prefactor for `invalidation.md`.

Deliverable when implemented: unit tests that assert the nests below. Code shapes below are the ones that **compile** (standalone harness). No parallel spine type: focus is `PathMut` / root path.

## Model (complete the composition)

Leave started at **C**. Origin type **includes C**:

```text
Out = Doll<CPath, Doll<BPath, APath>>
// = Result<CPath, Result<BPath, APath>>
// APath = root path (&mut Root). Bare terminal — no Result<APath, !>.
```

| stop | value | Result |
| --- | --- | --- |
| C | `Here(c)` | `Ok(c)` |
| B | `Up(Here(b))` | `Err(Ok(b))` |
| A | `Up(Up(a))` | `Err(Err(a))` — **a bare**, not `Here(a)` |

Every leave path holds a function `f` (as type-state). Always:

```text
non-root focus:  complete = f(Here(focus))
root focus:      complete = f_bare(focus)     // last Up holds APath; no Here
```

Start at C: `f = identity`.

```text
complete at C = id(Here(c)) = Here(c)
```

Each `into_parent`: laserbeam peel, then `f := |x| f(Up(x))` (compose one `Err`/`Up`).

```text
at B: f = Up
      complete = Up(Here(b))           // wrap Ok(b), then Err → Err(Ok(b))

at A: f = Up ∘ Up
      complete = Up(Up(a))             // bare root
```

One-level lift (Result language):

```text
lift : Result<BPath, T> → Result<CPath, Result<BPath, T>>   // = Err
// general:  Local → Outer  = Up
```

```text
f_B = lift_C ∘ Here = Err ∘ Ok     →  |b| Err(Ok(b))
f_A = lift_C ∘ lift_B              →  |a| Err(Err(a))   // bare A
```

Either: yes (`Here`/`Up` ≡ Ok/Err). Ok is inhabited where you can stop. Root bottoms out as bare `APath` inside the last `Up` — **no `!`**.

Root **as leave origin** (dispatch on `&mut Root` only): no doll leave machine; root only builds child paths with `from_fn` and matches the child’s `Doll`. Root **as terminal focus** of a deeper leave: `Terminal` wrap, `Up(…(root))`.

## Existing machinery

```rust
// laserbeam
pub struct PathMut<Node, Parent> { /* … */ }
impl<Node, Parent> PathMut<Node, Parent> {
    pub fn into_parent(self) -> Parent { self.parent }
    pub fn from_fn(parent, proj_mut, proj_ref) -> Self { /* … */ }
    pub fn get(&self) -> &Node { /* … */ }
    pub fn get_mut(&mut self) -> &mut Node { /* … */ }
}

// bind
impl<N, P> HasParent for PathMut<N, P> {
    type Parent = P;
    fn into_parent(self) -> P { PathMut::into_parent(self) }
}

// Place::Path for root is &mut Self; for a child, PathMut<Self, Parent::Path>
```

## Types and full impls (origin C)

Spine types (tests use stand-ins; production uses real `from_fn` aliases):

```rust
use core::marker::PhantomData;
use laserbeam::PathMut;

pub enum Doll<H, U> {
    Here(H),
    Up(U),
}

// Production:
// type APath<'a> = &'a mut Root;
// type BPath<'a> = PathMut<B, APath<'a>>;
// type CPath<'a> = PathMut<C, BPath<'a>>;
// type COut<'a>  = Doll<CPath<'a>, Doll<BPath<'a>, APath<'a>>>;

// Harness stand-ins (same structure):
pub struct Root;
pub struct BNode;
pub struct CNode;
pub type APath = Root;
pub type BPath = PathMut<BNode, APath>;
pub type CPath = PathMut<CNode, BPath>;
pub type COut = Doll<CPath, Doll<BPath, APath>>;
```

### Wrap type-state (= the function `f`)

```rust
/// f = identity. complete = Here(focus).
pub struct Id<Rest>(PhantomData<Rest>);

impl<Rest> Id<Rest> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

/// f = |x| outer(Up(x)). `Skipped` is the PathMut type peeled past (phantom).
pub struct ComposeUp<Skipped, Inner> {
    inner: Inner,
    _skipped: PhantomData<Skipped>,
}

impl<Skipped, Inner> ComposeUp<Skipped, Inner> {
    pub const fn new(inner: Inner) -> Self {
        Self {
            inner,
            _skipped: PhantomData,
        }
    }
}

/// Innermost at root: complete uses bare focus inside Up (no Here).
pub struct Terminal;
```

### LeavePath

```rust
pub struct LeavePath<P, F> {
    focus: P,
    f: F,
}
```

### Start at C (`f = id`)

```rust
pub fn leave_at_c(path: CPath) -> LeavePath<CPath, Id<Doll<BPath, APath>>> {
    LeavePath {
        focus: path,
        f: Id::new(),
    }
}
```

### At C: complete + into_parent

```rust
impl LeavePath<CPath, Id<Doll<BPath, APath>>> {
    /// f = id; complete = Here(c) = Ok(c)
    pub fn complete(self) -> COut {
        Doll::Here(self.focus)
    }

    /// peel C→B; f := Up ∘ id
    pub fn into_parent(self) -> LeavePath<BPath, ComposeUp<CPath, Id<APath>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp::new(Id::new()),
        }
    }
}
```

### At B: complete + into_parent

```rust
impl LeavePath<BPath, ComposeUp<CPath, Id<APath>>> {
    /// f(Here(b)) = Up(Here(b)) = Err(Ok(b))
    pub fn complete(self) -> COut {
        Doll::Up(Doll::Here(self.focus))
    }

    /// peel B→A (A root); f := Up ∘ Up with Terminal innermost
    pub fn into_parent(self) -> LeavePath<APath, ComposeUp<CPath, ComposeUp<BPath, Terminal>>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp::new(ComposeUp::new(Terminal)),
        }
    }
}
```

### At A (root): complete only

```rust
impl LeavePath<APath, ComposeUp<CPath, ComposeUp<BPath, Terminal>>> {
    /// f bare: Up(Up(a)) = Err(Err(a)) — no Here on root
    pub fn complete(self) -> COut {
        Doll::Up(Doll::Up(self.focus))
    }
}
```

No `into_parent` on root focus.

## Full impls (origin B — child of root)

When leave starts at B (e.g. `Outer` with parent root only):

```text
BOut = Doll<BPath, APath>
// Here(b) | Up(a)
```

```rust
pub type BOut = Doll<BPath, APath>;

pub fn leave_at_b(path: BPath) -> LeavePath<BPath, Id<APath>> {
    LeavePath {
        focus: path,
        f: Id::new(),
    }
}

impl LeavePath<BPath, Id<APath>> {
    pub fn complete(self) -> BOut {
        Doll::Here(self.focus)
    }

    pub fn into_parent(self) -> LeavePath<APath, ComposeUp<BPath, Terminal>> {
        LeavePath {
            focus: self.focus.into_parent(),
            f: ComposeUp::new(Terminal),
        }
    }
}

impl LeavePath<APath, ComposeUp<BPath, Terminal>> {
    pub fn complete(self) -> BOut {
        Doll::Up(self.focus)
    }
}
```

## Walk (origin C)

```text
leave_at_c(c)
  focus = c, f = Id
  complete → Here(c)

.into_parent()
  focus = b, f = ComposeUp<C, Id>
  complete → Up(Here(b))

.into_parent()
  focus = a (root), f = ComposeUp<C, ComposeUp<B, Terminal>>
  complete → Up(Up(a))
```

## Composition (function view)

```text
Out fixed: Doll<C, Doll<B, A>>

at C:  f_C = id
       complete = f_C(Here(c)) = Here(c)

into_parent: f_B = |x| f_C(Up(x)) = Up
at B:  complete = f_B(Here(b)) = Up(Here(b))

into_parent: f_A = |x| f_B(Up(x)) = Up∘Up
             but root uses Terminal so last step is bare Up(a) not Up(Here(a))
at A:  complete = Up(Up(a))
```

`Id` / `ComposeUp` / `Terminal` **are** those functions as types. `complete` bodies are the monomorphized applications. No `Option<fn>` at runtime.

## How root is special (concrete)

| | Non-root focus (`PathMut`) | Root focus (`&mut Root` / `APath`) |
| --- | --- | --- |
| In nest | can be `Here(path)` | only appears inside last `Up` |
| `complete` | `…Here(focus)` or as above | `…Up(focus)` via `Terminal` |
| `into_parent` | yes | no |
| Detect | `Parent` is `PathMut<_,_>` | `Parent` is `&mut T` (root `Place::Path`) |

Derive already treats root specially (`Path = &mut Self`). Same split here: `into_parent` impls that peel to `&mut T` install `Terminal`; peels between `PathMut`s install `Id` / nested `ComposeUp` with `Here` at the next stop.

Optional uniform `Result<APath, !>` would make every layer `f(Here(focus))` including A; not required. Prefer bare `APath`.

## Production aliases (bind / mercury style)

```rust
// Example only — real names from Place
type AppPath<'a> = &'a mut App;
type LayerPath<'a> = PathMut<Layer, AppPath<'a>>;
type NavPath<'a> = PathMut<Nav, LayerPath<'a>>;

// Leave started at NavPath — same shape as COut
type NavAscent<'a> = Doll<NavPath<'a>, Doll<LayerPath<'a>, AppPath<'a>>>;

// Leave started at LayerPath — same shape as BOut
type LayerAscent<'a> = Doll<LayerPath<'a>, AppPath<'a>>;
```

`leave_at` / `complete` / `into_parent` for those aliases are the same impl pattern as `leave_at_c` / `leave_at_b` with the corresponding type parameters. Derive emits one family per node path depth, or a small macro over depth.

## Naming

```rust
// Prefer path-keyed or node-keyed alias as needed — not the expanded Doll nest in APIs
type AscentOfNav<'a> = NavAscent<'a>;
// Dispatch: type Ascent<'a> = NavAscent<'a>;  // for Nav node
```

Match: `Doll::Here` / `Doll::Up` only.

## Tests (deliverable)

```rust
#[test]
fn c_stop_at_c() {
    let c = PathMut::new(PathMut::new(Root));
    assert!(matches!(leave_at_c(c).complete(), Doll::Here(_)));
}

#[test]
fn c_stop_at_b() {
    let c = PathMut::new(PathMut::new(Root));
    assert!(matches!(
        leave_at_c(c).into_parent().complete(),
        Doll::Up(Doll::Here(_))
    ));
}

#[test]
fn c_stop_at_root() {
    let c = PathMut::new(PathMut::new(Root));
    assert!(matches!(
        leave_at_c(c).into_parent().into_parent().complete(),
        Doll::Up(Doll::Up(Root))
    ));
}

#[test]
fn b_stop_at_b() {
    let b = PathMut::new(Root);
    assert!(matches!(leave_at_b(b).complete(), Doll::Here(_)));
}

#[test]
fn b_stop_at_root() {
    let b = PathMut::new(Root);
    assert!(matches!(leave_at_b(b).into_parent().complete(), Doll::Up(Root)));
}

#[test]
fn cout_type() {
    let c = PathMut::new(PathMut::new(Root));
    let _: COut = leave_at_c(c).into_parent().into_parent().complete();
}
```

All five behavioral tests + type assignment: must pass in the implementation crate.

## Ordered changes

### 1 — `Doll`, `Id`, `ComposeUp`, `Terminal`, `LeavePath`

### 2 — Full `leave_at_c` family (three `complete`, two `into_parent`) + tests for origin C

### 3 — Full `leave_at_b` family + tests for origin B

### 4 — Same pattern on real `PathMut::from_fn` + `&mut` root (bind test fixtures)

### 5 — Wire invalidation leave/kill to this; derive emits path-depth-specific impls or macro

## Rules

1. Origin nest includes start path: `Doll<C, Doll<B, A>>` with A root path bare.
2. Start `f = id`; `complete` at C is `Here(c)`.
3. Each `into_parent`: `PathMut::into_parent` + compose one `Up` into `f`.
4. At root focus: `Terminal`; `complete` is `Up(…(a))` without `Here(a)`.
5. Public `Here`/`Up` only; Result Ok/Err optional private synonym.
6. No `!` unless a later change forces uniform layers.
7. Every listed `complete` / `into_parent` has a full body; add depth only with a new impl + test.

## Relation to invalidation

Child returns `COut` / path-specific ascent. Parent matches `Here`/`Up`. Claim and posts separate. Root dispatch does not use `leave_at` on `&mut Root`; it only consumes the child’s doll.
