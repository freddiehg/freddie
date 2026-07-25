# Path peel + complete (nested Result pack)

Not done. Standalone. Prefactor for `invalidation.md`.

Build a leave path that peels with `into_parent` and finishes with `complete()`, always returning the same origin type `Out`. After two peels from C to A:

```rust
// Out = Result<BPath, Result<APath, Z>>
path.into_parent().into_parent().complete()  // Err(Ok(a_path)) inside Out

// Out = Result<BPath, APath>  (APath terminal bare)
path.into_parent().into_parent().complete()  // Err(a_path)
```

No dispatch, claim, posts, or handlers. Only the path + pack machine.

laserbeam stays `PathMut<Node, Parent>` with path-only `into_parent`. This lives in `bind` (or a tiny crate bind uses).

## Model

`Out` is a nested `Result` doll fixed at the start of the leave. Each peel moves the focus up one laserbeam parent and rewrites a **pack** value so `Pack::Out` stays that same `Out`. `complete()` is `pack.pack(focus)`.

```text
start leave at C (PathMut<C, BPath>), Out = Result<BPath, Result<APath, Z>>

into_parent  →  focus BPath, pack = AsHere,     Out unchanged
into_parent  →  focus APath, pack = AsUp…,    Out unchanged
complete()   →  Err(Ok(a_path)) : Out
```

One peel only:

```text
into_parent  →  focus BPath
complete()   →  Ok(b_path) : Out
```

## Types

```rust
use core::marker::PhantomData;

// --- Pack: focus P → Out ---

pub trait Pack<P> {
    type Out;
    fn pack(self, path: P) -> Self::Out;
}

/// This Result layer stops here: Ok(path).
pub struct AsHere<E>(PhantomData<E>);

impl<P, E> Pack<P> for AsHere<E> {
    type Out = Result<P, E>;
    fn pack(self, path: P) -> Result<P, E> {
        Ok(path)
    }
}

/// This Result layer was skipped: Err(inner.pack(path)).
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

/// Terminal stop: Out is the path value (no Result).
pub struct AsTerminal;

impl<P> Pack<P> for AsTerminal {
    type Out = P;
    fn pack(self, path: P) -> P {
        path
    }
}

// --- PeelPack: rewrite pack when focus PathMut → Parent; Out unchanged ---

pub trait PeelPack<Node, Parent>: Pack<laserbeam::PathMut<Node, Parent>> + Sized {
    type After: Pack<Parent, Out = Self::Out>;
    fn peel_pack(self) -> Self::After;
}

impl<Node, Parent, E> PeelPack<Node, Parent> for AsHere<Result<Parent, E>> {
    type After = AsUp<laserbeam::PathMut<Node, Parent>, AsHere<E>>;
    fn peel_pack(self) -> Self::After {
        AsUp(AsHere(PhantomData), PhantomData)
    }
}

impl<Node, Parent> PeelPack<Node, Parent> for AsHere<Parent> {
    type After = AsUp<laserbeam::PathMut<Node, Parent>, AsTerminal>;
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

// --- Leave path: focus + pack ---

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

    pub fn focus(&self) -> &P {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut P {
        &mut self.focus
    }
}

impl<Node, Parent, Pk> Path<laserbeam::PathMut<Node, Parent>, Pk>
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

/// Start a leave from a node PathMut. First peel; pack = AsHere; Out = Result<Parent, Rest>.
pub fn after_first_peel<Node, Parent, Rest>(
    path: laserbeam::PathMut<Node, Parent>,
) -> Path<Parent, AsHere<Rest>> {
    Path {
        focus: path.into_parent(),
        pack: AsHere(PhantomData),
    }
}
```

`Out` is not a separate type parameter on `Path`. It is `Pk::Out`. The pack **is** the wrap stack.

## API the caller writes

```rust
// C path: PathMut<C, BPath>, BPath = PathMut<B, APath>
// Out = Result<BPath, Result<APath, Z>>

type Out = Result<BPath, Result<APath, Z>>;

let out: Out = after_first_peel::<C, BPath, Result<APath, Z>>(c_path)
    .into_parent()
    .complete();
// Err(Ok(a_path))
```

`after_first_peel` is the first `into_parent` (C → B) plus installing `AsHere`. The second `.into_parent()` is B → A. Same as two peels then complete.

Bare terminal rest (`Out = Result<BPath, APath>`):

```rust
type Out = Result<BPath, APath>;

let out: Out = after_first_peel::<C, BPath, APath>(c_path)
    .into_parent()
    .complete();
// Err(a_path)
```

One peel only:

```rust
let out: Out = after_first_peel::<C, BPath, Result<APath, Z>>(c_path).complete();
// Ok(b_path)
```

## Worked types (no narrative)

```rust
// BPath = PathMut<B, APath>
// APath = PathMut<A, Z>   or bare A for terminal examples

// --- two Result layers, stop at A ---
// Out = Result<BPath, Result<APath, Z>>

// after_first_peel:
//   Path { focus: BPath, pack: AsHere<Result<APath, Z>> }
//   Pack::Out = Result<BPath, Result<APath, Z>>

// .into_parent()  (BPath = PathMut<B, APath>):
//   PeelPack on AsHere<Result<APath, Z>>
//   Path { focus: APath, pack: AsUp<BPath, AsHere<Z>> }
//   Pack::Out = Result<BPath, Result<APath, Z>>  (unchanged)

// .complete():
//   AsUp packs: Err(AsHere.pack(a_path)) = Err(Ok(a_path))

// --- bare rest, stop at A ---
// Out = Result<BPath, APath>

// after_first_peel:
//   Path { focus: BPath, pack: AsHere<APath> }

// .into_parent():
//   PeelPack on AsHere<APath>
//   Path { focus: APath, pack: AsUp<BPath, AsTerminal> }

// .complete():
//   Err(a_path)
```

## laserbeam (unchanged)

```rust
// existing
impl<Node, Parent> PathMut<Node, Parent> {
    pub fn into_parent(self) -> Parent {
        self.parent
    }
}
```

## Tests (bind)

```rust
#[test]
fn one_peel_is_ok() {
    // c: PathMut<C, PathMut<B, A>>
    let out = after_first_peel::<C, PathMut<B, A>, Result<A, ()>>(c).complete();
    assert!(matches!(out, Ok(_)));
}

#[test]
fn two_peels_nested_result_is_err_ok() {
    let out = after_first_peel::<C, PathMut<B, A>, Result<A, ()>>(c)
        .into_parent()
        .complete();
    assert!(matches!(out, Err(Ok(_))));
}

#[test]
fn two_peels_bare_rest_is_err() {
    let out = after_first_peel::<C, PathMut<B, A>, A>(c)
        .into_parent()
        .complete();
    assert!(matches!(out, Err(_)));
}

#[test]
fn three_peels_nested() {
    // c: PathMut<C, PathMut<B, PathMut<A, Z>>>
    // Out = Result<BPath, Result<APath, Result<Z, ()>>>
    let out = after_first_peel::<C, BPath, Result<APath, Result<Z, ()>>>(c)
        .into_parent()
        .into_parent()
        .complete();
    assert!(matches!(out, Err(Err(Ok(_)))));
}
```

## Ordered changes

### 1 — `Pack`, `AsHere`, `AsUp`, `AsTerminal` in `bind`

### 2 — `PeelPack` impls (nested rest + bare parent rest + recurse on `AsUp`)

### 3 — `Path`, `complete`, `into_parent`, `after_first_peel`

### 4 — unit tests above

No mercury / derive / Dispatch changes.

## Rules

1. `complete() -> Pk::Out` only; Out fixed for the leave.
2. `into_parent` peels laserbeam focus and `peel_pack`s; Out unchanged.
3. Nested rest: `AsHere<Result<Parent, E>>` peels to `AsUp<PathMut, AsHere<E>>`.
4. Bare rest: `AsHere<Parent>` peels to `AsUp<PathMut, AsTerminal>`.
5. laserbeam does not know about pack.

## Relation to invalidation

`invalidation.md` uses this leave machine for kill/normal leave and wraps `Out` in opaque `Ascent` for dispatch. This doc ships first and does not depend on claim, posts, or schedule.
